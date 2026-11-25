# SPDX-License-Identifier: MPL-2.0
"""Validate the clean release against project-side hashes, then smoke-test a copy."""
import hashlib
import json
import subprocess
import sys
import tempfile
import tomllib
import zipfile
from pathlib import Path

root = Path(__file__).resolve().parent.parent
version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
build = json.loads((root / 'reports/package.json').read_text(encoding='utf-8'))
name = build['archive_root']
archive = Path(build['zip'])
expected = {f['path']: f for f in build['files']}
allowed = set(build['allowed_runtime_paths'])
for file in expected:
    if file not in allowed and not file.startswith('licenses/'):
        raise SystemExit(f'Non-release file in package: {file}')
if not allowed.issubset(expected):
    raise SystemExit('Missing runtime file')
(root / '.tools').mkdir(exist_ok=True)
target = Path(tempfile.mkdtemp(prefix=f'portable-check-{version}-release-', dir=root / '.tools'))
with zipfile.ZipFile(archive) as z:
    bad = z.testzip()
    if bad:
        raise SystemExit(f'ZIP CRC failed: {bad}')
    if set(z.namelist()) != {f'{name}/{p}' for p in expected}:
        raise SystemExit('Archive content differs from clean release manifest')
    for info in z.infolist():
        if not (target / info.filename).resolve().is_relative_to(target.resolve()):
            raise SystemExit('Unsafe archive path')
    z.extractall(target)
package = target / name
for path, file in expected.items():
    actual = hashlib.sha256((package / path).read_bytes()).hexdigest()
    if actual != file['sha256']:
        raise SystemExit(f'SHA256 mismatch: {path}')

def check(flag, timeout=45):
    result = subprocess.run([str(package / 'sylva-vr.exe'), flag], cwd=package, timeout=timeout)
    if result.returncode != 0:
        raise SystemExit(f'Portable {flag} failed: {result.returncode}')

check('--self-test')
report = {'zip_crc': 'passed', 'release_allowlist': 'passed', 'sha256_files_checked': len(expected),
          'portable_engine_test': 'passed', 'archive_sha256': hashlib.sha256(archive.read_bytes()).hexdigest(),
          'archive_bytes': archive.stat().st_size, 'test_copy': str(package)}
if '--lifecycle' in sys.argv:
    check('--lifecycle-check')
    checked = json.loads((package / 'reports/lifecycle-check.json').read_text(encoding='utf-8'))
    if checked['result'].get('Ok') != 12:
        raise SystemExit('Lifecycle check count mismatch')
    report['portable_lifecycle_checks'] = 12
check('--feedback-check')
rendered = json.loads((package / 'reports/feedback-check.json').read_text(encoding='utf-8'))
if rendered['version'] != version or len(rendered['previews']) != 5:
    raise SystemExit('Portable feedback preview report mismatch')
report['portable_feedback_test'] = 'passed'
check('--neural-check', 90)
measured = json.loads((package / 'reports/neural-check.json').read_text(encoding='utf-8'))
if not any(p['neural_enabled'] and p['real_result_delivered'] for p in measured['passes']):
    raise SystemExit('Portable neural model produced no current prediction result')
report['portable_neural_test'] = 'passed'
(root / 'reports/package-check.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps(report, indent=2))
