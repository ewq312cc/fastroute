#!/usr/bin/env python3
"""pack.template.yaml -> pack.yaml: fills in the pins (Table 3-1 of the locked datasheet, through extract_pin_table.py) and the
SHA-256 of the example carrier board. Run from anywhere:

    python3 tools/build_pack.py [--datasheet DATASHEET.pdf]       (default: the file in workspace/cache/)

Writes pack.yaml next to the template. Needs `pdftotext`."""
import hashlib
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import extract_pin_table as pins  # noqa: E402


def datasheet():
    if "--datasheet" in sys.argv:
        return sys.argv[sys.argv.index("--datasheet") + 1]
    found = sorted((ROOT / "workspace/cache/DOC-ESP32S3-WROOM1-DS").glob("*.pdf"))
    if not found:
        sys.exit("the datasheet is not cached: run `hcp-manager --workspace workspace sources sync`, or give --datasheet")
    return str(found[0])


ps = pins.table(datasheet())
if len(ps) != 41:
    sys.exit(f"expected 41 pins, found {len(ps)}")
carrier = hashlib.sha256((ROOT / "example/carrier.kicad_pcb").read_bytes()).hexdigest()
text = (ROOT / "pack.template.yaml").read_text(encoding="utf-8")
text = text.replace("@@PINS@@", pins.yaml_lines(ps)).replace("@@CARRIER_SHA@@", carrier)
(ROOT / "pack.yaml").write_text(text, encoding="utf-8")
print(f"pack.yaml written: 41 pins, carrier.kicad_pcb sha256 {carrier[:12]}")
