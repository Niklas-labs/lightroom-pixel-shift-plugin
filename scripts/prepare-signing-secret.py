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
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        material = validate_material({
            'p12_base64': base64.b64encode(args.p12.read_bytes()).decode('ascii'),
            'p12_password': getpass.getpass('P12 export password: '),
        })
        private_write(args.output, (json.dumps(material) + '\n').encode())
    except (SigningError, OSError, ValueError):
        print('Could not prepare signing value; check input files, fields and unused output path.', file=sys.stderr)
        return 1
    print('Private signing value prepared. Import it into OpenBao, then remove this file.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
