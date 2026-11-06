#!/usr/bin/env python3
"""Run the project toolchain with developer paths removed from emitted code."""
import os
from pathlib import Path
import shlex
import sys

project = Path(__file__).resolve().parent.parent
env = os.environ.copy()
tools = Path(env.get('RUST_EXPLORER_TOOLCHAIN_ROOT', project / '.tools')).resolve()
env['RUSTUP_HOME'] = str(tools / 'rustup')
env['CARGO_HOME'] = str(tools / 'cargo')
env['PATH'] = str(tools / 'cargo/bin') + os.pathsep + env.get('PATH', '')
flags = (env['CARGO_ENCODED_RUSTFLAGS'].split('\x1f') if env.get('CARGO_ENCODED_RUSTFLAGS')
         else shlex.split(env.get('RUSTFLAGS', '')))
# rustc uses the last matching prefix. Map the wider home before the project.
# Encoded flags preserve spaces in paths without shell interpolation.
prefixes = [(str(Path.home()), '/user'), (str(project), '/rust-explorer'), (str(tools / 'cargo'), '/cargo'), (str(tools / 'rustup'), '/rustup')]
if env.get('TMPDIR'):
    prefixes.append((str(Path(env['TMPDIR']).resolve()), '/tmp'))
if env.get('CARGO_TARGET_DIR'):
    prefixes.append((str(Path(env['CARGO_TARGET_DIR']).resolve()), '/build'))
flags += ['--remap-path-prefix=' + source + '=' + target for source, target in prefixes]
env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(flags)
env.pop('RUSTFLAGS', None)
for variable in ('CFLAGS', 'CXXFLAGS'):
    values = shlex.split(env.get(variable, ''))
    values += ['-ffile-prefix-map=' + source + '=' + target for source, target in prefixes]
    values += ['-fdebug-prefix-map=' + source + '=' + target for source, target in prefixes]
    env[variable] = shlex.join(values)
env['CC_SHELL_ESCAPED_FLAGS'] = '1'
env.setdefault('MACOSX_DEPLOYMENT_TARGET', '14.0')
os.chdir(project)
os.execve(str(tools / 'cargo/bin/cargo'), ['cargo', *sys.argv[1:]], env)
