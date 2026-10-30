#!/usr/bin/env python3
"""Validate the shipped camera catalog against its Draft 2020-12 JSON Schema."""
import copy
import json
from pathlib import Path
from jsonschema import Draft202012Validator

root = Path(__file__).resolve().parents[1]
schema = json.loads((root / 'config/camera-profiles.schema.json').read_text())
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
config = json.loads((root / 'config/camera-profiles.json').read_text())
validator.validate(config)
for change in ['version', 'unknown', 'scale', 'offset', 'duplicate', 'null']:
    bad = copy.deepcopy(config)
    camera = bad['cameras'][0]
    if change == 'version': bad['schema_version'] = 2
    elif change == 'unknown': camera['typo'] = True
    elif change == 'scale': camera['pixel_shift']['four']['output_scale'] = 2
    elif change == 'offset': camera['pixel_shift']['four']['offsets'][0] = [0.5, 0]
    elif change == 'duplicate': camera['pixel_shift']['four']['offsets'][0] = [1, 1]
    else: camera['pixel_shift']['four'] = None
    assert not validator.is_valid(bad), change
print('Camera catalog and JSON Schema checks passed')
