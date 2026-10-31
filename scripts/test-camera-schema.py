#!/usr/bin/env python3
"""Validate the shipped camera catalog against its Draft 2020-12 JSON Schema."""
import copy
import json
from pathlib import Path
from jsonschema import Draft202012Validator
from camera_yaml import load_yaml

root = Path(__file__).resolve().parents[1]
schema = json.loads((root / 'config/camera-profiles.schema.json').read_text())
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
config = load_yaml((root / 'config/camera-profiles.yaml').read_text())
validator.validate(config)
for change in ['version', 'unknown', 'scale', 'offset', 'duplicate', 'null', 'rendering_exposure', 'rendering_matrix', 'rendering_unknown', 'rendering_null', 'normalization_white', 'normalization_unknown', 'normalization_null']:
    bad = copy.deepcopy(config)
    camera = bad['cameras'][0]
    if change == 'version': bad['schema_version'] = 2
    elif change == 'unknown': camera['typo'] = True
    elif change == 'scale': camera['pixel_shift']['four']['output_scale'] = 2
    elif change == 'offset': camera['pixel_shift']['four']['offsets'][0] = [0.5, 0]
    elif change == 'duplicate': camera['pixel_shift']['four']['offsets'][0] = [1, 1]
    elif change == 'rendering_exposure': camera['dng_rendering']['baseline_exposure_ev'] = 6
    elif change == 'rendering_matrix': camera['dng_rendering']['forward_matrix_a'] = [1,2]
    elif change == 'rendering_unknown': camera['dng_rendering']['typo'] = True
    elif change == 'rendering_null': camera['dng_rendering'] = None
    elif change == 'normalization_white': camera['raw_normalization']['white_level'] = 0
    elif change == 'normalization_unknown': camera['raw_normalization']['typo'] = True
    elif change == 'normalization_null': camera['raw_normalization'] = None
    else: camera['pixel_shift']['four'] = None
    assert not validator.is_valid(bad), change
print('Camera catalog and JSON Schema checks passed')

for text in ['schema_version: 1\nschema_version: 2', 'a: !unsupported value', 'a: 1\n---\na: 2']:
    try:
        load_yaml(text)
    except Exception:
        pass
    else:
        raise AssertionError('Invalid YAML accepted: ' + text)
assert load_yaml('x: on')['x'] == 'on'
