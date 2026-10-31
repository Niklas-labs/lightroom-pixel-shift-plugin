#!/usr/bin/env python3
"""Prepare a private JSON file for `bao kv put`; never print its contents."""
import argparse
import base64
import getpass
import json
from pathlib import Path
import sys

from ci_signing import SigningError, private_write, validate_material


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--p12', type=Path, required=True)
    parser.add_argument('--notary-key', type=Path, required=True)
    parser.add_argument('--key-id', required=True)
    parser.add_argument('--issuer-id', required=True)
    parser.add_argument('--identity-sha1', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        material = validate_material({
            'certificate_p12_base64': base64.b64encode(args.p12.read_bytes()).decode('ascii'),
            'certificate_password': getpass.getpass('P12 export password: '),
            'signing_identity_sha1': args.identity_sha1.replace(' ', '').replace(':', ''),
            'notary_key_p8': args.notary_key.read_text(),
            'notary_key_id': args.key_id,
            'notary_issuer_id': args.issuer_id,
        })
        private_write(args.output, (json.dumps(material) + '\n').encode())
    except (SigningError, OSError, ValueError):
        print('Could not prepare signing value; check input files, fields and unused output path.', file=sys.stderr)
        return 1
    print('Private signing value prepared. Import it into OpenBao, then remove this file.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
