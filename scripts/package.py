import os
import re
import shutil
import sys
import tarfile
import zipfile
from pathlib import Path

platform, ref = sys.argv[1:]
assert platform in ('linux-x86_64', 'windows-x86_64', 'macos-arm64', 'macos-x86_64')
name = f'quakewatch-{re.sub(r"[^a-zA-Z0-9._-]", "-", ref)}-{platform}'
dist = Path('dist/ci')
stage = dist / name
stage.mkdir(parents=True, exist_ok=True)
ext = '.exe' if platform.startswith('windows') else ''
for binary, directory in [('quakewatch', 'target'), ('quakewatch-gui', 'desktop/target')]:
    source = Path(directory) / 'release' / (binary + ext)
    shutil.copy2(source, stage / source.name)
for file in ('README.md',):
    shutil.copy2(file, stage / file)
for file in Path('desktop/vendor/xattr').glob('LICENSE-*'):
    shutil.copy2(file, stage / f'xattr-{file.name}')
if ext:
    with zipfile.ZipFile(dist / f'{name}.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
        for file in sorted(stage.iterdir()):
            archive.write(file, file.relative_to(dist))
else:
    with tarfile.open(dist / f'{name}.tar.gz', 'w:gz') as archive:
        archive.add(stage, arcname=name)
shutil.rmtree(stage)
