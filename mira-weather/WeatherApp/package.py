#!/usr/bin/env python3
"""Build Sky Glance: compile weather.rs, embed it with the phone companion, validate."""
from pathlib import Path
import base64, subprocess, sys, tempfile
root = Path(__file__).resolve().parent
sys.path.insert(0, str(root.parent / 'CompanionExample'))
from validate import validate_html

TITLE = 'Sky Glance'
output = Path(sys.argv[1] if len(sys.argv) > 1 else root.parent / 'SkyGlance.html')
with tempfile.TemporaryDirectory(prefix='sky-glance-') as folder:
    module = Path(folder) / 'weather.wasm'
    subprocess.run(['rustc', str(root / 'weather.rs'), '--edition=2021', '--target=wasm32-unknown-unknown',
                    '--crate-type=cdylib', '-C', 'opt-level=z', '-C', 'panic=abort', '-C', 'lto=fat',
                    '-C', 'codegen-units=1', '-C', 'link-arg=--no-entry', '-C', 'link-arg=--strip-all',
                    '-C', 'target-feature=-bulk-memory,+bulk-memory-opt,-reference-types',
                    '-C', 'link-arg=-zstack-size=8192', '-C', 'link-arg=--initial-memory=65536',
                    '-C', 'link-arg=--max-memory=65536', '-o', str(module)], check=True)
    encoded = base64.b64encode(module.read_bytes()).decode()
    companion = (root / 'companion.html').read_text(encoding='utf-8')
    html = ('<!doctype html><html><head><meta charset="utf-8">'
            '<meta name="viewport" content="width=device-width,initial-scale=1">\n'
            '<script type="application/mira-manifest+json">{"title":"' + TITLE + '"}</script>\n'
            '<title>' + TITLE + '</title></head><body>\n' + companion +
            '\n<script type="application/mira-wasm">' + encoded + '</script></body></html>')
    validate_html(html)
    output.write_text(html, encoding='utf-8')
print(output)
