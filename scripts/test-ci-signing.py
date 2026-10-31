#!/usr/bin/env python3
"""Credential-flow and publication gates; never needs Apple/OpenBao credentials."""
import base64
from contextlib import redirect_stdout
import hashlib
import importlib.util
import io
import json
import os
import ssl
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib import parse

import ci_signing as signing

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('package_plugin', ROOT / 'scripts/package-plugin.py')
packager = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packager)


CERT_DER = b'fixture public certificate DER'
CERT_PEM = ssl.DER_cert_to_PEM_cert(CERT_DER)
IDENTITY = hashlib.sha1(CERT_DER).hexdigest().upper()


def material():
    return {'p12_base64': base64.b64encode(b'fixture p12').decode(), 'p12_password': 'fixture password'}


class SigningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = {'RUNNER_TEMP': str(self.root),
                    'ACTIONS_ID_TOKEN_REQUEST_URL': 'https://pipelines.actions.githubusercontent.com/oidc?api-version=2.0',
                    'ACTIONS_ID_TOKEN_REQUEST_TOKEN': 'fixture-bearer'}
        self.directory = signing.credentials_directory(self.env)
        self.calls = []
        self.addCleanup(patch.stopall)
        patch.object(signing, 'CERTIFICATE_SHA256', hashlib.sha256(CERT_DER).hexdigest()).start()

    def fetch(self, fail_read=False, fail_revoke=False):
        def transport(url, headers, payload=None):
            self.calls.append((url, headers, payload))
            if (parse.urlsplit(url).hostname or '').endswith('.actions.githubusercontent.com'):
                return {'value': 'fixture-jwt'}
            if url.endswith('/login'):
                return {'auth': {'client_token': 'fixture-session'}}
            if url.endswith('/revoke-self'):
                if fail_revoke:
                    raise signing.SigningError('fixture revoke failed')
                return {}
            if fail_read:
                raise signing.SigningError('fixture read failed')
            return {'data': {'data': material()}}
        with redirect_stdout(io.StringIO()):
            signing.fetch_credentials(self.env, transport, lambda _: None)

    def test_oidc_audience_read_one_path_revoke_and_private_files(self):
        self.fetch()
        self.assertEqual(parse.parse_qs(parse.urlsplit(self.calls[0][0]).query)['audience'], [signing.BAO_ADDRESS])
        self.assertEqual(self.calls[1][0], signing.BAO_ADDRESS + '/v1/auth/token.actions.githubusercontent.com/login')
        self.assertEqual(self.calls[1][2], {'role': 'lightroom-pixel-shift-macos-signing', 'jwt': 'fixture-jwt'})
        self.assertEqual(self.calls[2][0], signing.BAO_ADDRESS + '/v1/personal/data/apple/signing-identities/2TF792U5Y6/developer-id-application/01162bea66636d78e09580172b23cf922d71fe04e2fc8879b827dc98c6091960')
        self.assertTrue(self.calls[-1][0].endswith('/revoke-self'))
        self.assertEqual(json.loads((self.directory / 'credentials.json').read_text()), material())
        self.assertEqual(self.directory.stat().st_mode & 0o777, 0o700)
        self.assertEqual((self.directory / 'credentials.json').stat().st_mode & 0o777, 0o600)
        self.assertNotIn('fixture-session', (self.directory / 'credentials.json').read_text())
        signing.cleanup(self.directory)
        self.assertFalse(self.directory.exists())

    def test_read_failure_revokes_and_revoke_failure_stops_signing(self):
        for kwargs in ({'fail_read': True}, {'fail_revoke': True}):
            with self.assertRaises(signing.SigningError):
                self.fetch(**kwargs)
            self.assertTrue(self.calls[-1][0].endswith('/revoke-self'))
            self.assertFalse(self.directory.exists())

    def test_rejects_stale_directory_untrusted_endpoint_and_plain_http(self):
        self.directory.mkdir()
        with self.assertRaises(FileExistsError):
            self.fetch()
        self.assertTrue(self.directory.is_dir())
        self.directory.rmdir()
        self.env['ACTIONS_ID_TOKEN_REQUEST_URL'] = 'https://untrusted.example/oidc'
        with self.assertRaises(signing.SigningError):
            self.fetch()
        self.assertFalse(self.directory.exists())
        self.assertEqual(self.calls, [])
        with self.assertRaises(signing.SigningError):
            signing.request_json('http://untrusted.example/', {})
        self.assertIsNone(signing.NoRedirect().redirect_request(None, None, 302, '', {}, 'https://elsewhere.example'))

    def test_schema_rejects_missing_and_malformed_material(self):
        for field in signing.FIELDS:
            data = material(); del data[field]
            with self.assertRaises(signing.SigningError): signing.validate_material(data)
        for field, value in [('p12_base64', '!'), ('p12_base64', ''), ('p12_password', 42), ('p12_password', '\0')]:
            data = material(); data[field] = value
            with self.assertRaises(signing.SigningError): signing.validate_material(data)
        data=material(); data['p12_password']=''
        self.assertEqual(signing.validate_material(data),data)
        wrapped=material(); wrapped['p12_base64']='\n'+wrapped['p12_base64']+'\n'
        self.assertEqual(signing.validate_material(wrapped),material())
        data['metadata']={'label':'not persisted'}
        self.assertEqual(set(signing.validate_material(data)),signing.FIELDS)

    def test_identity_is_derived_from_pinned_certificate_and_team(self):
        identities = '1) ' + IDENTITY + ' "Developer ID Application: Fixture (2TF792U5Y6)"'
        self.assertEqual(signing.select_identity(CERT_PEM,identities),IDENTITY)
        for certificates, listed in [('',identities),(CERT_PEM,identities.replace('2TF792U5Y6','WRONGTEAM')),
                                     (CERT_PEM,identities.replace(IDENTITY,'A'*40))]:
            with self.assertRaises(signing.SigningError): signing.select_identity(certificates,listed)

    def setup_binary(self):
        self.fetch()
        binary = self.root / 'sony-pixelshift'
        binary.write_bytes(b'fixture Mach-O bytes')
        binary.chmod(0o755)
        return binary

    def test_sign_imports_expected_identity_uses_runtime_timestamp_and_cleans_keychain(self):
        binary = self.setup_binary()
        calls = []
        def runner(command, label, **kwargs):
            calls.append(command)
            if 'create-keychain' in command:
                Path(command[-1]).touch()
            if 'find-identity' in command:
                return '1) ' + IDENTITY + ' "Developer ID Application: Fixture (2TF792U5Y6)"'
            if 'find-certificate' in command:
                return CERT_PEM
            if 'delete-keychain' in command:
                Path(command[-1]).unlink()
            return ''
        with redirect_stdout(io.StringIO()): signing.sign(binary, self.directory, runner)
        command = next(c for c in calls if c[0]=='codesign' and '--force' in c)
        self.assertIn('--timestamp', command)
        self.assertIn('runtime', command)
        self.assertEqual(command[command.index('--sign')+1], IDENTITY)
        self.assertTrue(any('delete-keychain' in c for c in calls))
        self.assertFalse((self.directory / 'certificate.p12').exists())
        self.assertFalse((self.directory / 'signing.keychain-db').exists())
        # Failed signing also removes the import and its private keychain.
        def failure(command,label,**kwargs):
            if command[0]=='codesign': raise signing.SigningError('fixture sign failure')
            return runner(command,label,**kwargs)
        with self.assertRaises(signing.SigningError): signing.sign(binary,self.directory,failure)
        self.assertFalse((self.directory / 'signing.keychain-db').exists())
        self.assertFalse((self.directory / 'certificate.p12').exists())

    def test_apple_acceptance_required_receipt_is_bound_to_binary(self):
        binary = self.setup_binary()
        with self.assertRaisesRegex(signing.SigningError,'Separate Apple notarization'):
            signing.notarize(binary,self.directory,lambda *a,**kw: self.fail('Must not invoke Apple without credentials'))
        signing.private_write(self.directory/'notary-credentials.json',json.dumps({
            'notary_key_p8':'-----BEGIN PRIVATE KEY-----\nfixture\n-----END PRIVATE KEY-----\n',
            'notary_key_id':'ABC123DE45','notary_issuer_id':'00000000-0000-0000-0000-000000000001'}).encode())
        for status in ('Invalid','In Progress','Accepted'):
            def runner(command,label,**kwargs):
                self.assertEqual(command[0:3], ['xcrun','notarytool','submit'])
                self.assertIn('--wait',command)
                self.assertTrue(Path(command[3]).exists())
                return json.dumps({'status':status,'id':'00000000-0000-0000-0000-000000000002'})
            if status != 'Accepted':
                with self.assertRaises(signing.SigningError): signing.notarize(binary,self.directory,runner)
            else:
                receipt=signing.notarize(binary,self.directory,runner)
                self.assertEqual(receipt['engine_sha256'],hashlib.sha256(binary.read_bytes()).hexdigest())
            self.assertFalse((self.directory / 'notary-helper.zip').exists())
            self.assertFalse(any(self.directory.glob('*.p8')))
        # Failed Apple status or mismatched binary cannot be labelled notarized.
        for receipt in [{'status':'Invalid'}, {'status':'Accepted','id':'00000000-0000-0000-0000-000000000002','engine_sha256':'bad'}]:
            with self.assertRaises(ValueError):
                packager.build_package(binary,'macos-universal',self.root/'dist','a'*40,{}, {},
                                       macos_signing='developer-id',notarization_receipt=receipt)

    def test_mask_escapes_workflow_commands_and_does_not_print_locally(self):
        output=io.StringIO()
        with patch.dict(os.environ,{'GITHUB_ACTIONS':'true'}), redirect_stdout(output):
            signing.mask('a%\r\nb')
        self.assertIn('::add-mask::a%25%0D%0Ab',output.getvalue())
        output=io.StringIO()
        with patch.dict(os.environ,{'GITHUB_ACTIONS':'false'}), redirect_stdout(output): signing.mask('private')
        self.assertEqual(output.getvalue(),'')

    def test_workflow_scopes_oidc_to_signing_environment(self):
        import yaml
        workflow=yaml.safe_load((ROOT/'.github/workflows/package.yml').read_text())
        for name,job in workflow['jobs'].items():
            if name=='macos':
                self.assertEqual(job['permissions']['id-token'],'write')
                self.assertEqual(job['environment'],'macos-signing')
                signed=[s for s in job['steps'] if 'run' in s and 'ci_signing.py fetch' in s['run']]
                self.assertNotIn('if',signed[0])
                self.assertFalse(any('ad-hoc' in s.get('name','') for s in job['steps']))
                self.assertFalse(any('ci_signing.py notarize' in s.get('run','') for s in job['steps']))
                self.assertEqual(job.get('env',{}).get('MACOS_SIGNING_ENABLED'),None)
                cleanup=[s for s in job['steps'] if 'run' in s and 'ci_signing.py cleanup' in s['run']]
                self.assertEqual(cleanup[0]['if'],'always()')
                for step in job['steps']:
                    command=step.get('run','')
                    if '--receipt ' in command or '--notarization-receipt ' in command:
                        self.assertIn('$RUNNER_TEMP/pixelshift-signing/notarization-receipt.json',command)
                        self.assertNotIn('target/notarization-receipt',command)

            else:
                self.assertNotEqual(job.get('permissions',{}).get('id-token'),'write')



if __name__=='__main__': unittest.main()
