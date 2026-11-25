# SPDX-License-Identifier: MPL-2.0
"""Build a clean end-user release from an allowlist; keep reports in the project."""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tomllib
import zipfile
from pathlib import Path

root = Path(__file__).resolve().parent.parent
version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
parser = argparse.ArgumentParser()
parser.add_argument('name', nargs='?', default=f'Sylva-VR-{version}')
args = parser.parse_args()
if Path(args.name).name != args.name or args.name in ('.', '..'):
    raise SystemExit('Output must be a simple package directory name.')

# Separate pristine release output from previously used portable installations.
# Never package runtime/, settings.json or files found in an old release folder.
dest = root / 'dist' / 'release' / args.name
if dest.exists():
    raise SystemExit(f'Existing release preserved; choose a new output name: {dest}')
metadata = json.loads(subprocess.check_output(
    ['cargo', 'metadata', '--format-version', '1', '--locked',
     '--filter-platform', 'x86_64-pc-windows-msvc'], cwd=root, encoding='utf-8'))
target = Path(metadata['target_directory'])
cargo_home = Path(os.environ.get('CARGO_HOME', Path.home() / '.cargo')).resolve()
sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], cwd=root,
                                       encoding='utf-8').strip())
build_paths = {Path.home(): '/home/builder', cargo_home: '/cargo', sysroot: '/rust',
               root: '/sylva', target: '/target'}
flags = [f'--remap-path-prefix={path}={build_paths[path]}'
         for path in sorted(build_paths, key=lambda p: len(str(p)))]
config = 'target.x86_64-pc-windows-msvc.rustflags=' + json.dumps(flags, ensure_ascii=False)
subprocess.run(['cargo', 'build', '--release', '--locked', '--config', config], cwd=root, check=True)
executable = target / 'release/sylva-vr.exe'
binary = executable.read_bytes().lower()
for path in build_paths:
    for spelling in (str(path), path.as_posix()):
        if any(spelling.encode(enc).lower() in binary for enc in ('utf-8', 'utf-16-le')):
            raise SystemExit('Local build path embedded in executable. Check compiler flags.')
dest.mkdir(parents=True)

def copy(source, target=None):
    output = dest / (target or source)
    output.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(root / source, output)

runtime_files = {
    executable: 'sylva-vr.exe',
    'vendor/openvr_api.dll': 'openvr_api.dll',
    'bindings/actions.json': 'bindings/actions.json',
    'bindings/oculus_touch.json': 'bindings/oculus_touch.json',
    'bindings/pico_controller.json': 'bindings/pico_controller.json',
    'assets/branding/sylva.png': 'assets/branding/sylva.png',
    'models/sentence-model.safetensors': 'models/sentence-model.safetensors',
}
for name in ['msime-pinyin.db', 'msime-english.db', 'msime-bigram.bin',
             'msime-trigram.bin', 'msime-dictionary-manifest.json', 'msime-scowl_Copyright.txt']:
    runtime_files[f'data/{name}'] = f'data/{name}'
for source, target in runtime_files.items():
    copy(source, target)

copy('packaging/THIRD_PARTY.txt', 'licenses/THIRD_PARTY.txt')
copy('LICENSE', 'licenses/Sylva/MPL-2.0.txt')
copy('vendor/OPENVR-LICENSE.txt', 'licenses/OpenVR/LICENSE.txt')
shutil.copytree(root / 'data/licenses', dest / 'licenses/dictionary')
copy('data/README.md', 'licenses/dictionary/UPSTREAM-NOTICE.md')
copy('data/msime-scowl_Copyright.txt', 'licenses/dictionary/msime-scowl_Copyright.txt')
copy('data/msime-dictionary-manifest.json', 'licenses/dictionary/msime-dictionary-manifest.json')
for name in ['LICENSE-APACHE-2.0.txt', 'attribution.txt', 'manifest.json']:
    copy(f'models/{name}', f'licenses/neural-model/{name}')

components = []
for package in metadata['packages']:
    if not package['source']:
        continue
    name, ver = package['name'], package['version']
    components.append(f"{name} {ver} — {package['license']}\n"
                      f"Source: https://static.crates.io/crates/{name}/{name}-{ver}.crate\n")
    folder = Path(package['manifest_path']).parent
    for item in folder.iterdir():
        if item.name.lower().startswith(('license', 'licence', 'copying', 'notice', 'copyright')):
            output = dest / 'licenses' / f'{name}-{ver}' / item.name
            output.parent.mkdir(parents=True, exist_ok=True)
            if item.is_dir():
                shutil.copytree(item, output)
            else:
                shutil.copy2(item, output)
(dest / 'licenses/components.txt').write_text('\n'.join(components), encoding='utf-8')

files = [{'path': file.relative_to(dest).as_posix(), 'bytes': file.stat().st_size,
          'sha256': hashlib.sha256(file.read_bytes()).hexdigest()}
         for file in sorted(dest.rglob('*')) if file.is_file()]
# Only runtime files and license material are allowed. No blanket directory copies.
allowed = set(runtime_files.values())
assert all(f['path'] in allowed or f['path'].startswith('licenses/') for f in files)
archive = root / 'dist' / f'{args.name}.zip'
pending = archive.with_suffix('.zip.pending')
with zipfile.ZipFile(pending, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9, strict_timestamps=False) as z:
    for file in files:
        z.write(dest / file['path'], f"{args.name}/{file['path']}")
pending.replace(archive)
report = {'package': str(dest), 'archive_root': args.name, 'zip': str(archive),
          'zip_bytes': archive.stat().st_size, 'unpacked_bytes': sum(f['bytes'] for f in files),
          'allowed_runtime_paths': sorted(allowed), 'files': files}
(root / 'reports').mkdir(exist_ok=True)
(root / 'reports/package.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
print(json.dumps({k: v for k, v in report.items() if k not in ('files', 'allowed_runtime_paths')}, ensure_ascii=False, indent=2))
