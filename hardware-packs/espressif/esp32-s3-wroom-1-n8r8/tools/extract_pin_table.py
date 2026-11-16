#!/usr/bin/env python3
"""Reads Table 3-1 (Pin Definitions) of the locked module datasheet and writes the `pins:` list of
the pack, so that no pin number, name or function is typed from memory.

    extract_pin_table.py DATASHEET.pdf            print the pins as YAML
    extract_pin_table.py DATASHEET.pdf --check PACK.yaml
                                                  exit 1 if the pack's pins differ from the table

Needs `pdftotext` (poppler-utils) and PyYAML only for --check. Pin classes are decided here, each
from a statement of the same datasheet (see CLASS_SOURCES); anything else is `gpio`.
"""
import json
import re
import subprocess
import sys

TABLE_PAGES = ("11", "12")  # printed and PDF page numbers agree: Table 3-1 is on pages 11 and 12
ROW = re.compile(r"^(\S+)(?: ([a-z]))? (\d+) (I/O/T|P|I)(?:\s+(.*))?$")

# Which statement of the datasheet decides a pin's class:
#   Table 3-1 type P            power or ground (by name)
#   Table 3-1 pin EN            reset (the pin enables / resets the chip)
#   Table 4-1 / Section 4       strapping pins: GPIO0, GPIO3, GPIO45, GPIO46
#   Table 3-1 note b            IO35, IO36, IO37 are connected to the Octal SPI PSRAM (this module has it:
#                               Table 1-1, ESP32-S3-WROOM-1-N8R8: PSRAM 8 MB (Octal SPI))
#   Table 3-1 functions         USB_D-, USB_D+ on GPIO19 / GPIO20; U0TXD, U0RXD on pins 37 / 36
CLASS_SOURCES = {
    "strapping": {"IO0", "IO3", "IO45", "IO46"},
    "reserved": {"IO35", "IO36", "IO37"},
}


def pdf_text(pdf):
    out = subprocess.run(["pdftotext", "-raw", "-f", TABLE_PAGES[0], "-l", TABLE_PAGES[1], pdf, "-"], capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit("pdftotext failed: " + out.stderr.strip()[:200])
    return out.stdout


def rows(text):
    """The table rows, with a wrapped function list put back together."""
    lines = [l.strip() for l in text.splitlines()]
    start = next(i for i, l in enumerate(lines) if l.startswith("Name No. Type"))
    items = []
    for l in lines[start + 1:]:
        if l.startswith(("Cont", "Espressif", "Submit", "ESP32-S3-WROOM", "3 Pin Definitions", "Table 3-1", "Name No.")):
            continue
        if re.match(r"^[a-c] (P|For)\b", l) or l.startswith(("a P:", "b For", "c For")):
            break
        items.append(l)
    pins, i = [], 0
    while i < len(items):
        m = ROW.match(items[i])
        if not m:
            i += 1
            continue
        name, mark, number, typ, fn = m.groups()
        j = i + 1
        extra = []
        while j < len(items) and not ROW.match(items[j]):
            extra.append(items[j])
            j += 1
        if name == "EN":
            parts = ["EN"]  # the lines under EN describe the pin (High: on ... Low: off ...), they name no function
        else:
            parts = [fn] if fn else []
            parts += extra  # a long function list wraps onto the lines under the row
        pins.append({"number": number, "name": name, "mark": mark, "type": typ, "raw": " ".join(parts), "functions": [f.strip() for f in " ".join(parts).split(",") if f.strip()]})
        i = j
    return pins


def classify(p):
    n = p["name"]
    if p["type"] == "P":
        return "power" if n == "3V3" else "ground"
    if n == "EN":
        return "reset"
    if n in CLASS_SOURCES["strapping"]:
        return "strapping"
    if n in CLASS_SOURCES["reserved"]:
        return "reserved"
    if any(f in ("USB_D-", "USB_D+") for f in p["functions"]):
        return "usb"
    if n in ("TXD0", "RXD0"):
        return "uart"
    return "gpio"


def layout_pages(pdf):
    """The whitespace-normalised text of the two pages, as hcp-manager reads them (pdftotext -layout)."""
    pages = []
    for pg in TABLE_PAGES:
        out = subprocess.run(["pdftotext", "-layout", "-f", pg, "-l", pg, pdf, "-"], capture_output=True, text=True)
        pages.append(" ".join(out.stdout.split()))
    return pages


def table(pdf):
    ps = rows(pdf_text(pdf))
    pages = layout_pages(pdf)
    for p in ps:
        p["class"] = classify(p)
        # the evidence of a row: the whole row if the page text has it in one piece, else its first cells
        mark = f" {p['mark']}" if p.get("mark") else ""
        head = f"{p['name']}{mark} {p['number']} {p['type']}"
        full = f"{head} {p['raw']}" if p["name"] != "EN" else head
        for n, page_text in enumerate(pages):
            # a row that ends in a hyphen is joined to the next line by hcp-manager's text normalisation: quote only its first cells
            whole = full in page_text and not full.endswith("-")
            if whole or head in page_text:
                p["page"] = int(TABLE_PAGES[n])
                p["quote"] = full if whole else head
                break
        else:
            sys.exit(f"row of pin {p['number']} not found in the page text")
    return ps


def yaml_lines(ps):
    out = []
    for p in ps:
        fns = ", ".join(p["functions"])
        quote = p["quote"].replace('"', '\\"')
        ev = f'evidence: [{{ source_id: SRC-ESP32S3-WROOM1-DS, locator: {{ page: {p["page"]}, table: "Table 3-1. Pin Definitions" }}, quote: "{quote}", basis: explicit }}]'
        out.append(f'      - {{ number: "{p["number"]}", name: {p["name"]}, class: {p["class"]}, functions: [{fns}], {ev} }}')
    return "\n".join(out)


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    ps = table(args[0])
    if len(ps) != 41:
        sys.exit(f"expected 41 pins in Table 3-1, found {len(ps)}")
    if "--check" in args:
        import yaml
        pack = yaml.safe_load(open(args[args.index("--check") + 1], encoding="utf-8"))
        have = [(p["number"], p["name"], p["class"], p.get("functions", [])) for p in pack["devices"][0]["pins"]]
        want = [(p["number"], p["name"], p["class"], p["functions"]) for p in ps]
        if have != want:
            for h, w in zip(have, want):
                if h != w:
                    print("pack:", h, "\ndoc: ", w)
            sys.exit(1)
        print("the pins of the pack are Table 3-1 of the datasheet (41 pins)")
    else:
        print(yaml_lines(ps))
