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
import ssl
import shutil
import subprocess
import sys
from urllib import error, parse, request
import uuid
import zipfile

BAO_ADDRESS = 'https://bao.kalix.cluster.rosenstein.app'
AUTH_PATH = 'token.actions.githubusercontent.com'
ROLE = 'lightroom-pixel-shift-macos-signing'
SECRET_PATH = 'personal/data/apple/signing-identities/2TF792U5Y6/developer-id-application/01162bea66636d78e09580172b23cf922d71fe04e2fc8879b827dc98c6091960'
CERTIFICATE_SHA256 = SECRET_PATH.rsplit('/', 1)[1]
TEAM_ID = '2TF792U5Y6'
FIELDS = {'p12_base64', 'p12_password'}


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


def safe_endpoint_detail(raw):
    # Auth errors can embed JWT claims. Report only known field names/categories.
    try:
        errors = json.loads(raw).get('errors', [])
        message = ' '.join(e for e in errors if isinstance(e,str)).lower()
    except (ValueError, AttributeError, TypeError):
        return 'response withheld'
    fields = {'repository', 'repository_id', 'repository_owner_id', 'ref', 'ref_type',
              'event_name', 'workflow_ref', 'sub', 'iss', 'aud'}
    claims = [name for name in re.findall(r'claim [\"\']([a-z_]+)[\"\']', message) if name in fields]
    if claims:
        return 'JWT claim validation failed: ' + ', '.join(sorted(set(claims)))
    for word, category in [('audience','JWT audience validation failed'),('issuer','JWT issuer validation failed'),
                           ('signature','JWT signature validation failed'),('expired','JWT has expired')]:
        if word in message:
            return category
    if 'role' in message and any(word in message for word in ['not found','could not be found','unknown']):
        return 'requested Bao role was not found'
    return 'response withheld'


def claim_mismatches(jwt):
    # This is diagnostic only; Bao verifies the signature and authorizes access.
    try:
        payload = jwt.split('.')[1]
        claims = json.loads(base64.urlsafe_b64decode(payload + '='*((-len(payload))%4)))
        expected = {'iss':'https://token.actions.githubusercontent.com',
                    'sub':'repo:Niklas-labs/lightroom-pixel-shift-plugin:environment:macos-signing',
                    'repository':'Niklas-labs/lightroom-pixel-shift-plugin','repository_id':'1438976219',
                    'repository_owner_id':'343997774','ref':'refs/heads/main','ref_type':'branch',
                    'workflow_ref':'Niklas-labs/lightroom-pixel-shift-plugin/.github/workflows/package.yml@refs/heads/main'}
        mismatches = [key for key,value in expected.items() if claims.get(key)!=value]
        audience=claims.get('aud')
        if not (audience==BAO_ADDRESS or isinstance(audience,list) and BAO_ADDRESS in audience):
            mismatches.append('aud')
        if claims.get('event_name') not in ('push','workflow_dispatch'):
            mismatches.append('event_name')
        return ', '.join(sorted(mismatches)) or 'none'
    except (ValueError, TypeError, IndexError, AttributeError):
        return 'unavailable'


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
        detail=safe_endpoint_detail(exc.read(32768))
        raise SigningError(f'Credential endpoint returned HTTP {exc.code}; {detail}') from None
    except (error.URLError, TimeoutError, ValueError, OSError):
        raise SigningError('Credential endpoint failed; details withheld') from None


def validate_material(value):
    if not isinstance(value, dict) or not FIELDS.issubset(value):
        raise SigningError('Signing record requires p12_base64 and p12_password')
    # Ignore unrelated record metadata; never persist it or interpret it as notarization credentials.
    value = {key: value[key] for key in FIELDS}
    if any(not isinstance(v, str) or '\0' in v for v in value.values()):
        raise SigningError('Signing fields must be strings without NUL bytes')
    value['p12_base64'] = ''.join(value['p12_base64'].split())
    try:
        decoded = base64.b64decode(value['p12_base64'], validate=True)
    except (ValueError, binascii.Error):
        raise SigningError('Invalid P12 base64 encoding') from None
    if not decoded:
        raise SigningError('The P12 is empty')
    return value


def load_notary_material(directory):
    path = directory / 'notary-credentials.json'
    if not path.is_file():
        raise SigningError('Separate Apple notarization credentials are not configured')
    value = json.loads(path.read_text())
    fields = {'notary_key_p8', 'notary_key_id', 'notary_issuer_id'}
    if not isinstance(value, dict) or set(value) != fields or any(not isinstance(v,str) or not v or '\0' in v for v in value.values()):
        raise SigningError('Invalid separate notarization credentials')
    try:
        uuid.UUID(value['notary_issuer_id'])
    except ValueError:
        raise SigningError('Invalid notarization issuer ID') from None
    pem = value['notary_key_p8'].strip()
    if not re.fullmatch(r'[A-Z0-9]{10}', value['notary_key_id']) or not pem.startswith('-----BEGIN PRIVATE KEY-----') or not pem.endswith('-----END PRIVATE KEY-----'):
        raise SigningError('Invalid notarization API key')
    for field in value.values():
        mask(field)
    return value


def select_identity(certificates, identities):
    # The record path identifies the certificate by its DER SHA-256 fingerprint.
    # Derive codesign's SHA-1 identity from that certificate, not its display name.
    expected = None
    for pem in re.findall(r'-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----', certificates, re.S):
        der = ssl.PEM_cert_to_DER_cert(pem)
        if hashlib.sha256(der).hexdigest() == CERTIFICATE_SHA256:
            expected = hashlib.sha1(der).hexdigest().upper()
    if expected is None:
        raise SigningError('Imported P12 does not contain the certificate identified by the Bao record path')
    available = re.findall(r'([A-Fa-f0-9]{40})\s+"(Developer ID Application:[^"]+)"', identities)
    if not any(fp.upper() == expected and name.endswith('(' + TEAM_ID + ')') for fp, name in available):
        raise SigningError('Expected valid Developer ID Application identity and Apple team are absent')
    return expected


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
        try:
            login = transport(BAO_ADDRESS + '/v1/auth/' + AUTH_PATH + '/login', {}, {'role': ROLE, 'jwt': jwt})
        except SigningError as exc:
            print('OIDC expected-claim mismatches (names only): ' + claim_mismatches(jwt))
            raise SigningError('OpenBao OIDC login failed: ' + str(exc)) from None
        token = login['auth']['client_token']
        if not isinstance(token, str) or not token:
            raise SigningError('Missing OpenBao session token')
        masker(token)
        try:
            try:
                response = transport(BAO_ADDRESS + '/v1/' + SECRET_PATH, {'X-Vault-Token': token})
            except SigningError as exc:
                raise SigningError('OpenBao signing record read failed: ' + str(exc)) from None
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
    private_write(certificate, base64.b64decode(material['p12_base64'], validate=True))
    try:
        runner(['security', 'create-keychain', '-p', password, str(keychain)], 'Creating temporary keychain')
        runner(['security', 'set-keychain-settings', '-lut', '21600', str(keychain)], 'Configuring temporary keychain')
        runner(['security', 'unlock-keychain', '-p', password, str(keychain)], 'Unlocking temporary keychain')
        runner(['security', 'import', str(certificate), '-k', str(keychain), '-P', material['p12_password'],
                '-T', '/usr/bin/codesign', '-T', '/usr/bin/security'], 'Importing Developer ID certificate')
        runner(['security', 'set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', password,
                str(keychain)], 'Configuring signing key access')
        identities = runner(['security', 'find-identity', '-v', '-p', 'codesigning', str(keychain)], 'Checking signing identity')
        certificates = runner(['security', 'find-certificate', '-a', '-p', str(keychain)], 'Reading imported public certificates')
        expected = select_identity(certificates, identities)
        runner(['codesign', '--force', '--sign', expected, '--keychain', str(keychain), '--options', 'runtime',
                '--timestamp', str(binary)], 'Developer ID signing')
        runner(['codesign', '--verify', '--strict', '--all-architectures', '--verbose=2', str(binary)], 'Verifying signature')
    finally:
        certificate.unlink(missing_ok=True)
        if keychain.exists():
            runner(['security', 'delete-keychain', str(keychain)], 'Deleting temporary keychain')
    print('Universal helper signed with Developer ID and secure timestamp.')


def notarize(binary, directory, runner=run):
    material = load_notary_material(directory)
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
    parser.add_argument('command', choices=['fetch', 'sign', 'notarize', 'cleanup'])
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--receipt', type=Path)
    args = parser.parse_args()
    try:
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
