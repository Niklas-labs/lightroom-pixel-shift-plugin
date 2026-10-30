"""Safe camera-catalog YAML loading for build/schema tooling, never plugin runtime."""
import re
import yaml


class CameraLoader(yaml.SafeLoader):
    pass


# Match YAML 1.2 boolean semantics used by the Rust reader: on/off/yes/no are strings.
CameraLoader.yaml_implicit_resolvers = {
    key: [(tag, pattern) for tag, pattern in entries
          if tag != 'tag:yaml.org,2002:bool']
    for key, entries in yaml.SafeLoader.yaml_implicit_resolvers.items()
}
CameraLoader.add_implicit_resolver('tag:yaml.org,2002:bool',
                                 re.compile(r'^(?:true|True|TRUE|false|False|FALSE)$'),
                                 list('tTfF'))


def unique_mapping(loader, node):
    result = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node)
        if not isinstance(key, str):
            raise ValueError('Camera YAML mapping keys must be strings')
        if key in result:
            raise ValueError(f'Duplicate YAML key: {key}')
        result[key] = loader.construct_object(value_node)
    return result


CameraLoader.add_constructor('tag:yaml.org,2002:map', unique_mapping)


def load_yaml(text):
    return yaml.load(text, Loader=CameraLoader)
