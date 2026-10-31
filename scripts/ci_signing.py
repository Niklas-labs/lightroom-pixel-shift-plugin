#!/usr/bin/env python3
"""GitHub OIDC -> OpenBao -> ephemeral Developer ID signing and notarization.

No third-party Python packages, long-lived Bao tokens or GitHub secrets required.
Never print HTTP response bodies, credentials or subprocess command arguments.
"""
import argparse
import base64
import binascii
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
from urllib import error, parse, request
import uuid
import zipfile

BAO_ADDRESS = 'https://bao.kalix.cluster.rosenstein.app'
AUTH_PATH = 'github-pixelshift'
ROLE = 'macos-signing'
SECRET_PATH = 'pixelshift-ci/data/macos-signing'
FIELDS = {'certificate_p12_base64', 'certificate_password', 'signing_identity_sha1',
          'notary_key_p8', 'notary_key_id', 'notary_issuer_id'}


class SigningError(Exception):
    pass


def mask(value):
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        return
    # GitHub workflow-command escaping, including multiline PEM values.
    for text in [value, *value.splitlines()]:
        if text:
            text = text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print('::add-mask::' + text, flush=True)


class NoRedirect(request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def request_json(url, headers, payload=None):
    if parse.urlsplit(url).scheme != 'https':
        raise SigningError('Credential transport requires HTTPS')
    data = None if payload is None else json.dumps(payload).encode()
    headers = dict(headers, Accept='application/json')
    if data is not None:
        headers['Content-Type'] = 'application/json'
    try:
        req = request.Request(url, data=data, headers=headers)
        with request.build_opener(NoRedirect()).open(req, timeout=30) as response:
            raw = response.read(2 * 1024 * 1024 + 1)
        if len(raw) > 2 * 1024 * 1024:
            raise SigningError('Credential endpoint response exceeded size limit')
        return json.loads(raw) if raw else {}
    except error.HTTPError as exc:
        raise SigningError(f'Credential endpoint returned HTTP {exc.code}; response withheld') from None
    except (error.URLError, TimeoutError, ValueError, OSError):
        raise SigningError('Credential endpoint failed; details withheld') from None


def validate_material(value):
    if not isinstance(value, dict) or set(value) != FIELDS:
        raise SigningError('Signing value has missing or unexpected fields')
    if any(not isinstance(v, str) or not v or '\0' in v for v in value.values()):
        raise SigningError('Signing fields must be nonempty strings without NUL bytes')
    try:
        decoded = base64.b64decode(value['certificate_p12_base64'], validate=True)
        uuid.UUID(value['notary_issuer_id'])
    except (ValueError, binascii.Error):
        raise SigningError('Invalid certificate encoding or notarization issuer ID') from None
    if not decoded or not re.fullmatch(r'[a-fA-F0-9]{40}', value['signing_identity_sha1']):
        raise SigningError('Invalid certificate data or SHA-1 signing identity')
    if not re.fullmatch(r'[A-Z0-9]{10}', value['notary_key_id']):
        raise SigningError('Invalid notarization API key ID')
    pem = value['notary_key_p8'].strip()
    if not pem.startswith('-----BEGIN PRIVATE KEY-----') or not pem.endswith('-----END PRIVATE KEY-----'):
        raise SigningError('Notarization key must be a PKCS#8 PEM private key')
    return value


def private_write(path, data):
    # Do not follow/overwrite existing paths. The enclosing directory is private.
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb') as out:
        out.write(data)


def credentials_directory(env):
    root = env.get('RUNNER_TEMP')
    if not root or not Path(root).is_absolute():
        raise SigningError('RUNNER_TEMP must be an absolute directory')
    return Path(root).resolve() / 'pixelshift-signing'


def fetch_credentials(env, transport=request_json, masker=mask):
    directory = credentials_directory(env)
    directory.mkdir(mode=0o700)  # A stale directory must never be silently reused.
    try:
        endpoint = env['ACTIONS_ID_TOKEN_REQUEST_URL']
        bearer = env['ACTIONS_ID_TOKEN_REQUEST_TOKEN']
        parsed = parse.urlsplit(endpoint)
        if parsed.scheme != 'https' or not parsed.hostname or not parsed.hostname.endswith('.actions.githubusercontent.com'):
            raise SigningError('Unexpected GitHub OIDC request endpoint')
        query = [(k, v) for k, v in parse.parse_qsl(parsed.query) if k != 'audience']
        query.append(('audience', BAO_ADDRESS))
        oidc = transport(parse.urlunsplit(parsed._replace(query=parse.urlencode(query))),
                         {'Authorization': 'Bearer ' + bearer})
        jwt = oidc['value']
        if not isinstance(jwt, str) or not jwt:
            raise SigningError('Missing GitHub OIDC token')
        masker(jwt)
        login = transport(BAO_ADDRESS + '/v1/auth/' + AUTH_PATH + '/login', {}, {'role': ROLE, 'jwt': jwt})
        token = login['auth']['client_token']
        if not isinstance(token, str) or not token:
            raise SigningError('Missing OpenBao session token')
        masker(token)
        try:
            response = transport(BAO_ADDRESS + '/v1/' + SECRET_PATH, {'X-Vault-Token': token})
            material = validate_material(response['data']['data'])
            for value in material.values():
                masker(value)
            private_write(directory / 'credentials.json', json.dumps(material).encode())
        finally:
            # No token is persisted in files, GITHUB_ENV, outputs, artifacts or caches.
            transport(BAO_ADDRESS + '/v1/auth/token/revoke-self', {'X-Vault-Token': token}, {})
        print('Fetched signing material; OpenBao session revoked.')
    except BaseException:
        shutil.rmtree(directory)
        raise


def load_material(directory):
    material = validate_material(json.loads((directory / 'credentials.json').read_text()))
    for value in material.values():
        mask(value)
    return material


def run(command, label, timeout=120):
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired):
        raise SigningError(f'{label} could not finish; command details withheld') from None
    if result.returncode:
        raise SigningError(f'{label} failed (exit {result.returncode}); tool output withheld')
    return result.stdout


def sign(binary, directory, runner=run):
    material = load_material(directory)
    keychain = directory / 'signing.keychain-db'
    certificate = directory / 'certificate.p12'
    password = secrets.token_urlsafe(32)
    mask(password)
    private_write(certificate, base64.b64decode(material['certificate_p12_base64'], validate=True))
    try:
        runner(['security', 'create-keychain', '-p', password, str(keychain)], 'Creating temporary keychain')
        runner(['security', 'set-keychain-settings', '-lut', '21600', str(keychain)], 'Configuring temporary keychain')
        runner(['security', 'unlock-keychain', '-p', password, str(keychain)], 'Unlocking temporary keychain')
        runner(['security', 'import', str(certificate), '-k', str(keychain), '-P', material['certificate_password'],
                '-T', '/usr/bin/codesign', '-T', '/usr/bin/security'], 'Importing Developer ID certificate')
        runner(['security', 'set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', password,
                str(keychain)], 'Configuring signing key access')
        identities = runner(['security', 'find-identity', '-v', '-p', 'codesigning', str(keychain)], 'Checking signing identity')
        expected = material['signing_identity_sha1'].upper()
        available = re.findall(r'([A-Fa-f0-9]{40})\s+"(Developer ID Application:[^"]+)"', identities)
        if not any(fingerprint.upper() == expected for fingerprint, _ in available):
            raise SigningError('Expected Developer ID Application identity is absent or invalid')
        runner(['codesign', '--force', '--sign', expected, '--keychain', str(keychain), '--options', 'runtime',
                '--timestamp', str(binary)], 'Developer ID signing')
        runner(['codesign', '--verify', '--strict', '--all-architectures', '--verbose=2', str(binary)], 'Verifying signature')
    finally:
        certificate.unlink(missing_ok=True)
        if keychain.exists():
            runner(['security', 'delete-keychain', str(keychain)], 'Deleting temporary keychain')
    print('Universal helper signed with Developer ID and secure timestamp.')


def notarize(binary, directory, runner=run):
    material = load_material(directory)
    key = directory / ('AuthKey_' + material['notary_key_id'] + '.p8')
    archive = directory / 'notary-helper.zip'
    private_write(key, material['notary_key_p8'].encode())
    try:
        # Notarize the signed helper before packaging. The final plugin manifest
        # can then truthfully record acceptance without rewriting submitted code.
        with zipfile.ZipFile(archive, 'x', compression=zipfile.ZIP_DEFLATED) as zipped:
            zipped.write(binary, arcname=binary.name)
        output = runner(['xcrun', 'notarytool', 'submit', str(archive), '--key', str(key), '--key-id',
                         material['notary_key_id'], '--issuer', material['notary_issuer_id'], '--wait',
                         '--timeout', '20m', '--output-format', 'json'], 'Apple notarization', timeout=1500)
        record = json.loads(output)
        if record.get('status') != 'Accepted':
            raise SigningError('Apple did not accept the notarization submission; publication stopped')
        # Emit only nonsecret acceptance evidence for the packaging gate.
        return {'status': 'Accepted', 'id': str(uuid.UUID(record['id'])),
                'engine_sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
    finally:
        key.unlink(missing_ok=True)
        archive.unlink(missing_ok=True)


def cleanup(directory):
    if directory.is_symlink():
        raise SigningError('Refusing a symlinked signing directory')
    if directory.exists():
        keychain = directory / 'signing.keychain-db'
        if keychain.exists():
            run(['security', 'delete-keychain', str(keychain)], 'Deleting temporary keychain')
        shutil.rmtree(directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['mode', 'fetch', 'sign', 'notarize', 'cleanup'])
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--receipt', type=Path)
    args = parser.parse_args()
    try:
        if args.command == 'mode':
            if os.environ.get('MACOS_SIGNING_ENABLED', '') not in ('', 'false', 'true'):
                raise SigningError('MACOS_SIGNING_ENABLED must be true, false, or unset')
            return 0
        directory = credentials_directory(os.environ)
        if args.command == 'fetch':
            fetch_credentials(os.environ)
        elif args.command == 'cleanup':
            cleanup(directory)
        else:
            if not args.binary or not args.binary.is_file():
                raise SigningError('A built universal binary is required')
            if args.command == 'sign':
                sign(args.binary.resolve(), directory)
            else:
                if not args.receipt:
                    raise SigningError('A notarization receipt path is required')
                receipt = notarize(args.binary.resolve(), directory)
                private_write(args.receipt, (json.dumps(receipt) + '\n').encode())
                print('Apple accepted helper notarization: ' + receipt['id'])
        return 0
    except (SigningError, KeyError, TypeError, ValueError, OSError):
        # Do not let a traceback display exception payloads containing key material.
        # SigningError messages above are constructed solely from nonsecret labels.
        exc = sys.exc_info()[1]
        print(str(exc) if isinstance(exc, SigningError) else 'Signing setup failed; details withheld', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
