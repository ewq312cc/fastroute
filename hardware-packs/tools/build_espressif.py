#!/usr/bin/env python3
"""Builds the Espressif chip and module packs of hardware-packs/ from the locked documents of hardware-packs/espressif/workspace/.

    python3 hardware-packs/tools/build_espressif.py [--only ID,...]

Nothing is typed from memory: every quotation is looked up on its page (or in its HTML text) of the cached document before it is
written, every pin number and name of a module comes out of the pin table of its datasheet, and a rule that a document does not
support is not written. A fact that no document gives stays `unknown` and says what is missing.

Writes, per pack, <vendor>/<family>/<device>/<variant>/<version>/ with hcp.yaml, sources.yaml and pack.yaml (the manifest), seals
the manifest with `hcp-manager release seal` and (with fastroute) the Chinese pack report. Needs pdftotext, PyYAML, hcp-manager.
"""
import copy
import glob
import html
import unicodedata
import os
import pathlib
import re
import subprocess
import sys

import yaml

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import pdfq  # noqa: E402

HP = HERE.parent                      # hardware-packs/
REPO = HP.parent
ROOT = HP / "espressif"
WS = ROOT / "workspace"
VERSION = "0.1.0"
TODAY = "2026-11-16"


def tool(name):
    env = os.environ.get(name.upper().replace("-", "_") + "_BIN")
    if env:
        return env
    found = [REPO / "target" / d / name for d in ("release", "debug") if (REPO / "target" / d / name).exists()]
    if found:
        return str(max(found, key=lambda f: f.stat().st_mtime))  # the newest build
    sys.exit(f"{name} not found: cargo build -p {name}, or set {name.upper().replace('-', '_')}_BIN")


# ---- the documents ------------------------------------------------------------------------------------------------------------

INV = {d["document_id"]: d for d in yaml.safe_load((WS / "sources.yaml").read_text())["documents"]}


def nfkc(s):
    return unicodedata.normalize("NFKC", " ".join(s.split()))


def html_text(path):
    t = pathlib.Path(path).read_text(errors="replace")
    t = re.sub(r"<script.*?</script>|<style.*?</style>", "", t, flags=re.S)
    t = re.sub(r"<(?:br|/p|/tr|/h\d|/li|/div|/td|/th)[^>]*>", " ", t)
    t = re.sub(r"<[^>]+>", " ", t)
    return " ".join(html.unescape(t).split())


class Doc:
    def __init__(self, doc_id):
        self.id = doc_id
        self.entry = INV[doc_id]
        files = [f for f in sorted(glob.glob(str(WS / "cache" / doc_id / "*"))) if not f.endswith("meta.json")]
        if not files:
            sys.exit(f"{doc_id} is not cached: hcp-manager --workspace {WS} sources sync")
        self.path = files[0]
        self.pdf = self.path.endswith(".pdf")
        self.plain = self.path.endswith((".dxf", ".txt", ".md"))   # read as text, as hcp-manager does
        self._html = None

    def ok(self, quote, page=None):
        if self.pdf:
            ps = pdfq.pages(self.path)
            return page is not None and 1 <= page <= len(ps) and nfkc(quote) in nfkc(ps[page - 1])
        if self.plain:
            return nfkc(quote) in nfkc(pathlib.Path(self.path).read_text(encoding="latin-1"))
        if self._html is None:
            self._html = html_text(self.path)
        return nfkc(quote) in nfkc(self._html)

    def sent(self, prefix, n=1):
        """The text of an HTML page from `prefix` to the end of its n-th sentence, exactly as the page has it (special characters included)."""
        if self._html is None:
            self._html = html_text(self.path)
        i = self._html.index(prefix)
        end = i
        for _ in range(n):
            end = self._html.index(". ", end) + 1
        return self._html[i:end]

    def find(self, quote):
        return pdfq.find(self.path, quote) if self.pdf else []

    def ev(self, src, quote, page=None, table=None, section=None, figure=None, basis="explicit"):
        if not self.ok(quote, page):
            where = f"page {page}" if self.pdf else "the page"
            sys.exit(f"QUOTE NOT FOUND in {self.id} {where}: {quote!r}" + (f" (found on pages {self.find(quote)})" if self.pdf and self.find(quote) else ""))
        loc = {}
        if page is not None:
            loc["page"] = page
        if table:
            loc["table"] = table
        if section:
            loc["section"] = section
        if figure:
            loc["figure"] = figure
        return {"source_id": src, "locator": loc, "quote": quote, "basis": basis}


DOCS = {}


def doc(doc_id):
    if doc_id not in DOCS:
        DOCS[doc_id] = Doc(doc_id)
    return DOCS[doc_id]


def source(doc_id, source_id, applies_to, note=None):
    e = INV[doc_id]
    s = {"source_id": source_id, "document_id": doc_id, "title": e["title"], "version": e["version"], "level": e["level"], "url": e["url"],
         "retrieved": str(e["retrieved"]), "sha256": e["sha256"], "applies_to": list(applies_to)}
    if note:
        s["note"] = note
    return s


def inventory_entries(doc_ids):
    out = []
    for i in doc_ids:
        e = copy.deepcopy(INV[i])
        out.append(e)
    return {"sources_version": "1.0", "documents": out}


# ---- module pin tables --------------------------------------------------------------------------------------------------------

ROW = re.compile(r"(?<![A-Za-z0-9_])(3V3|[A-Za-z][A-Za-z0-9_]*) (\d+(?:,\d+)*) (P|I/O/T|I/O|I|O|-)(?= )")


def module_pins(d, pages, start_marker, end_marker, name_fix=None, expect=None):
    """The rows of a datasheet's pin table: name, number(s), type and the function text up to the next row."""
    text = " ".join(pdfq.pages(d.path)[p - 1] for p in pages)
    i = text.index(start_marker) + len(start_marker)
    text = text[i:text.index(end_marker, i)]
    for junk in ("Cont’d on next page", "Table 3 – cont’d from previous page", "Table 3-1 – cont’d from previous page"):
        text = text.replace(junk, " ")
    text = re.sub(r"Espressif Systems \d+ [A-Za-z0-9 &\-]+ Datasheet v[0-9.]+ Submit Documentation Feedback", " ", text)
    text = re.sub(r"3 Pin Definitions", " ", text)
    text = re.sub(r"Name No\. Type1? Function", " ", text)
    ms = list(ROW.finditer(text))
    pins = []
    for k, m in enumerate(ms):
        name, nos, typ = m.groups()
        fn = text[m.end(): ms[k + 1].start() if k + 1 < len(ms) else len(text)].strip()
        for no in nos.split(","):
            real = (name_fix or {}).get(name, name)
            pins.append({"number": no, "name": real, "table_name": name, "type": typ, "text": fn, "row": f"{name} {nos} {typ}"})
    if expect is not None and sorted(int(p["number"]) for p in pins) != list(range(1, expect + 1)):
        sys.exit(f"the pin table of {d.id} did not parse into pins 1..{expect}: found {sorted(int(p['number']) for p in pins)}")
    return pins


def pin_entry(d, src, p, cls, page_of, table):
    quote = p["row"]
    page = page_of(p)
    if not d.ok(quote, page):
        sys.exit(f"pin row not found on page {page}: {quote!r}")
    fn = [] if p["type"] in ("-",) else [f.strip() for f in re.split(r",\s*", re.sub(r"\(.*?\)", "", p["text"])) if f.strip() and not f.strip().startswith(("High", "Low", "Note", "See"))]
    if p["name"] == "EN":
        fn = ["EN"]
    e = {"number": p["number"], "name": p["name"], "class": cls}
    if fn:
        e["functions"] = fn
    e["evidence"] = [d.ev(src, quote, page, table=table)]
    return e


# ---- helpers for the packs ----------------------------------------------------------------------------------------------------


def rule(rid, kind, mod, title, subject=None, params=None, evidence=None, **extra):
    r = {"id": rid, "kind": kind, "modifiability": mod, "title": title}
    if subject is not None:
        r["subject"] = subject
    if params is not None:
        r["params"] = params
    if evidence:
        r["evidence"] = evidence
    r.update(extra)
    return r


def write_yaml(path, value, header=""):
    path.parent.mkdir(parents=True, exist_ok=True)
    text = yaml.safe_dump(value, sort_keys=False, allow_unicode=True, width=130, default_flow_style=False)
    path.write_text(header + text, encoding="utf-8")


def pack_dir(pid):
    return HP / pid / VERSION


def manifest(pid, kind, part, deps, docs, package=None, revision=None):
    v, f, dev, var = pid.split("/")
    ident = {"vendor": v, "family": f, "device": dev, "variant": var, "part_number": part}
    if revision:
        ident["revision"] = revision
    if package:
        ident["package"] = package
    return {
        "manifest_version": "1.0", "id": pid, "kind": kind, "identity": ident, "version": VERSION, "content": "full", "review": "draft",
        "compatibility": {"hcp_schema_version": "1.1", "ir_version": "1.0", "min_app_version": "0.1.14"},
        "dependencies": [{"id": i, "kind": k, "version": VERSION} for i, k in deps],
        "license": {
            "pack": "GPL-3.0-or-later",
            "documents": [{"document_id": d, "redistribution": "reference_only", "note": "Espressif document: only its URL, version, SHA-256, page locators and short quotations are kept"} for d in docs],
            "note": "The pack states facts with short quotations and page locators; the original documents are not part of it.",
        },
    }


def write_pack(pid, kind, part, hcp, deps, docs, package=None, revision=None, upstream_digest=None):
    d = pack_dir(pid)
    for old in ("hcp.yaml", "pack.yaml", "sources.yaml"):
        if (d / old).exists():
            (d / old).unlink()
    write_yaml(d / "hcp.yaml", hcp, "# Generated by hardware-packs/tools/build_espressif.py from the locked documents of hardware-packs/espressif/workspace/.\n# Edit the builder, not this file.\n")
    write_yaml(d / "sources.yaml", inventory_entries(docs))
    write_yaml(d / "pack.yaml", manifest(pid, kind, part, deps, docs, package, revision))
    report(d, [pack_dir(dep) / "hcp.yaml" for dep, _ in deps])
    subprocess.run([tool("hcp-manager"), "release", "seal", str(d)], check=True, capture_output=True)
    return d


def report(d, upstream_files):
    """The readable (Chinese) report of the pack as it is read once it is resolved with its upstream packs."""
    fr = tool("fastroute")
    src = d / "hcp.yaml"
    tmp = d / ".resolved.yaml"
    if upstream_files:
        args = [fr, "constraints", "resolve", str(src)] + [a for u in upstream_files for a in ("--upstream", str(u))] + ["-o", str(tmp), "--force"]
        out = subprocess.run(args, capture_output=True, text=True)
        if out.returncode != 0:
            sys.exit(f"{d}: the pack does not resolve:\n{out.stderr[-800:]}")
        src = tmp
    out = subprocess.run([fr, "constraints", "check", str(src), "-q"], capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"{d}: the pack is not valid:\n{out.stderr[-800:]}")
    (d / "reports").mkdir(exist_ok=True)
    out = subprocess.run([fr, "constraints", "report", str(src), "-o", str(d / "reports" / "pack-report.zh.md"), "--lang", "zh", "--force", "-q"], capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"{d}: no report:\n{out.stderr[-800:]}")
    if tmp.exists():
        tmp.unlink()


def digest(pid):
    out = subprocess.run([tool("hcp-manager"), "release", "digest", str(pack_dir(pid) / "hcp.yaml")], check=True, capture_output=True, text=True).stdout.strip()
    return out


INHERITED_KINDS = ("strapping", "boot_pin", "reset_pin", "electrical")


def inheritable(chip_pid):
    """The chip rules a module takes over: the facts about the chip's own pins and supply. The circuits of a chip-down design (blocks,
    their required_block rules) and the layout rules are not the module's: it has its own, on its own pins."""
    rules = yaml.safe_load(open(pack_dir(chip_pid) / "hcp.yaml"))["rules"]
    return [r["id"] for r in rules if r["kind"] in INHERITED_KINDS and r["modifiability"] != "unknown"]


def pack_meta(pid, kind, title, note, language="en", upstream=None):
    m = {"id": pid, "kind": kind, "title": title, "version": VERSION, "language": language, "created": TODAY,
         "extractor": {"kind": "mixed", "tool": "hardware-packs/tools/build_espressif.py: pin tables read from the datasheet, rules written by an AI assistant from the locked documents; every quotation is checked against its page"},
         "review": {"status": "draft", "note": note}}
    if upstream:
        m["upstream"] = upstream
    return m


REVIEW_NOTE = ("Draft: nobody has reviewed this pack. Facts quote the locked documents; what the documents do not say is `unknown` and "
               "says what is missing. The antenna keep-out of a module is drawn only in a figure and is `unknown` here.")


# ---- ESP32 -------------------------------------------------------------------------------------------------------------------

ESP32_CHIP = "espressif/esp32/esp32/d0wd-v3"
ESP32_MOD = "espressif/esp32/esp32-wroom-32e/n4"


def build_esp32_chip():
    dev = "DEV-ESP32-D0WD-V3"
    ds, sch = doc("DOC-ESP32-DS"), doc("DOC-ESP32-HDG-SCH")
    S, G = "SRC-ESP32-DS", "SRC-ESP32-HDG-SCH"
    pins_def = [("VDDA", 1, "power", "VDDA 1 P", 14), ("VDD3P3", 3, "power", "VDD3P3 3 P", 14), ("CHIP_PU", 9, "reset", "CHIP_PU 9 I", 14), ("MTDI", 18, "strapping", "MTDI 18 I/O", 14),
                ("VDD3P3_RTC", 19, "power", "VDD3P3_RTC 19 P", 14), ("MTDO", 21, "strapping", "MTDO 21 I/O", 14), ("GPIO2", 22, "strapping", "GPIO2 22 I/O", 15),
                ("GPIO0", 23, "strapping", "GPIO0 23 I/O", 15), ("GPIO5", 34, "strapping", "GPIO5 34 I/O", 15)]
    pins = [{"number": str(n), "name": name, "class": cls, "evidence": [ds.ev(S, q, pg, table="Table 2-1. Pin Overview")]} for name, n, cls, q, pg in pins_def]
    pn = {name: str(n) for name, n, *_ in pins_def}
    strap = [("GPIO0", "GPIO0 Pull-up 1", "high"), ("GPIO2", "GPIO2 Pull-down 0", "low"), ("MTDI", "MTDI Pull-down 0", "low"), ("MTDO", "MTDO Pull-up 1", "high"), ("GPIO5", "GPIO5 Pull-up 1", "high")]
    rules = []
    for name, q, lvl in strap:
        rules.append(rule(f"C-ESP32C-STRAP-{name}", "strapping", "fixed", f"{name} is a strapping pin: its default level at reset is {lvl}",
                          {"device": dev, "pin": pn[name]}, {"sampled_at": "reset", "default_level": lvl, "external_drive_allowed": True},
                          [ds.ev(S, q, 22, table="Table 3-1. Default Configuration of Strapping Pins")]))
    rules.append(rule("C-ESP32C-BOOT-GPIO0", "boot_pin", "fixed", "GPIO0 and GPIO2 both low select the Joint Download Boot mode",
                      {"device": dev, "pin": pn["GPIO0"]}, {"boot_level": "low", "mode": "Joint Download Boot (SDIO or UART download); GPIO2 must be low as well"},
                      [ds.ev(S, "GPIO0 and GPIO2 control the boot mode after the reset is released.", 23, section="3.1 Chip Boot Mode Control"),
                       ds.ev(S, "SPI Boot Mode 1 Any value Joint Download Boot Mode 2 0 0", 23, table="Table 3-3. Chip Boot Mode Control")]))
    rules.append(rule("C-ESP32C-RESET", "reset_pin", "fixed", "CHIP_PU low powers the chip off; it must not be left floating", {"device": dev, "pin": pn["CHIP_PU"]}, {"active_level": "low"},
                      [ds.ev(S, "Low: Off; the chip shuts down", 14, table="Table 2-1. Pin Overview"), ds.ev(S, "Note: Do not leave the CHIP_PU pin floating.", 14, table="Table 2-1. Pin Overview")]))
    rules.append(rule("C-ESP32C-ELECTRICAL-SUPPLY", "electrical", "fixed", "Analog and RTC supply pins take 2.3 V to 3.6 V", {"device": dev},
                      {"quantity": "VDDA, VDD3P3 and VDD3P3_RTC supply voltage", "condition": "as stated for the pins in Table 2-1 Pin Overview", "min_v": 2.3, "max_v": 3.6},
                      [ds.ev(S, "VDDA 1 P Analog power supply (2.3 V ∼ 3.6 V)", 14, table="Table 2-1. Pin Overview"), ds.ev(S, "VDD3P3_RTC 19 P Input power supply for RTC IO (2.3 V ∼ 3.6 V)", 14, table="Table 2-1. Pin Overview")]))
    blocks = [
        {"id": "BLK-ESP32C-RESET", "title": "RC delay circuit on CHIP_PU (usually 10 kΩ and 1 µF)", "kind": "reset_circuit", "modifiability": "configurable", "device": dev,
         "components": [{"role": "pull_up", "kind": "resistor", "min_count": 1, "value_ohm": 10000}, {"role": "delay", "kind": "capacitor", "min_count": 1, "min_value_nf": 1000}],
         "connects": [{"pin": pn["CHIP_PU"], "net_pattern": "*"}],
         "evidence": [sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")]},
        {"id": "BLK-ESP32C-BOOT", "title": "A way to pull GPIO0 low (button or jumper) to enter download mode", "kind": "boot_button", "modifiability": "configurable", "device": dev,
         "components": [{"role": "button", "kind": "switch", "min_count": 1}], "connects": [{"pin": pn["GPIO0"], "net_pattern": "*"}],
         "evidence": [ds.ev(S, "SPI Boot Mode 1 Any value Joint Download Boot Mode 2 0 0", 23, table="Table 3-3. Chip Boot Mode Control", basis="derived")]},
        {"id": "BLK-ESP32C-CRYSTAL", "title": "External 40 MHz crystal (compulsory)", "kind": "generic", "modifiability": "required", "device": dev,
         "components": [{"role": "crystal", "kind": "other", "min_count": 1}],
         "evidence": [sch.ev(G, "External crystal clock source (Compulsory)", section="Clock Source"), sch.ev(G, "The ESP32 firmware only supports 40 MHz crystal.", section="External Crystal Clock Source (Compulsory)")]},
    ]
    rules += [
        rule("C-ESP32C-NEED-RESET", "required_block", "configurable", "An RC delay circuit on CHIP_PU is advised: usually 10 kΩ and 1 µF, adjusted to the supply", {"block": "BLK-ESP32C-RESET"},
             evidence=[sch.ev(G, "it is advised to add an RC delay circuit at the CHIP_PU pin.", section="Chip Power-up and Reset Timing"), sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")], severity="warning"),
        rule("C-ESP32C-NEED-BOOT", "required_block", "derived", "Download mode needs GPIO0 pulled low: a button, a jumper or a programmer's output", {"block": "BLK-ESP32C-BOOT"},
             evidence=[ds.ev(S, "SPI Boot Mode 1 Any value Joint Download Boot Mode 2 0 0", 23, table="Table 3-3. Chip Boot Mode Control", basis="derived")], derived_from=["C-ESP32C-BOOT-GPIO0"], severity="warning"),
        rule("C-ESP32C-NEED-CRYSTAL", "required_block", "required", "The chip needs an external crystal, and the firmware supports 40 MHz only", {"block": "BLK-ESP32C-CRYSTAL"},
             evidence=[sch.ev(G, "External crystal clock source (Compulsory)", section="Clock Source"), sch.ev(G, "The ESP32 firmware only supports 40 MHz crystal.", section="External Crystal Clock Source (Compulsory)")]),
        rule("C-ESP32C-EN-RC-VALUES", "required_block", "unknown", "The final values of the CHIP_PU RC circuit", {"block": "BLK-ESP32C-RESET"},
             missing="The guideline says `usually R = 10 kΩ and C = 1 μF` and that the values have to be adjusted to the power supply and the power-up and reset timing of the project. No document gives the final values: the project decides and tests them."),
    ]
    hcp = {
        "hcp_version": "1.1",
        "pack": pack_meta(ESP32_CHIP, "chip", "ESP32 (ESP32-D0WD-V3, QFN48): chip-level constraints", REVIEW_NOTE),
        "sources": [source("DOC-ESP32-DS", S, [dev]), source("DOC-ESP32-HDG-SCH", G, [dev])],
        "devices": [{"id": dev, "vendor": "Espressif", "part_number": "ESP32-D0WD-V3", "package": "QFN48", "pins": pins}],
        "peripheral_blocks": blocks, "rules": rules,
    }
    write_pack(ESP32_CHIP, "chip", "ESP32-D0WD-V3", hcp, [], ["DOC-ESP32-DS", "DOC-ESP32-HDG-SCH"], package="QFN48")


def build_esp32_module():
    dev = "DEV-ESP32-WROOM-32E-N4"
    ds, sch = doc("DOC-ESP32-WROOM32E-DS"), doc("DOC-ESP32-HDG-SCH")
    S, G = "SRC-ESP32-WROOM32E-DS", "SRC-ESP32-HDG-SCH"
    raw = module_pins(ds, [11, 12], "Name No. Type1 Function", "1 P: power supply", name_fix={"IO163": "IO16"}, expect=38)
    cls_strap = {"IO0", "IO2", "IO12", "IO15", "IO5"}

    def cls(p):
        n = p["name"]
        return "power" if n == "3V3" else "ground" if n == "GND" else "reset" if n == "EN" else "nc" if n == "NC" else "strapping" if n in cls_strap else "uart" if n in ("TXD0", "RXD0") else "gpio"

    def page_of(p):
        return 12 if int(p["number"]) >= 28 or (int(p["number"]) == 38) else 11

    def table_page(p):
        for pg in (11, 12):
            if ds.ok(p["row"], pg):
                return pg
        sys.exit(f"pin row not on page 11 or 12: {p['row']}")

    pins = [pin_entry(ds, S, p, cls(p), table_page, "Table 3: Pin Definitions") for p in raw]
    by = {p["name"]: [q["number"] for q in raw if q["name"] == p["name"]] for p in raw}
    no = {n: v[0] for n, v in by.items()}
    T = "Table 3: Pin Definitions"
    rules = []
    for gnd in by["GND"]:
        rules.append(rule(f"C-ESP32M-GND-{gnd}", "fixed_net", "fixed", f"Pin {gnd} is ground", {"device": dev}, {"pin": gnd, "net_pattern": "GND*"}, [ds.ev(S, f"GND {gnd} P Ground", table_page({"row": f"GND {gnd} P"}) if False else (12 if gnd == "38" else 11), table=T)]))
    rules.append(rule("C-ESP32M-NC", "nc_pin", "fixed", "Pins 17 to 22 and 32 are not connected on the module (the flash pins GPIO6 to GPIO11 are not led out)", {"device": dev},
                      {"pins": by["NC"], "reason": "GPIO6 to GPIO11 of the chip are connected to the SPI flash inside the module and are not led out"},
                      [ds.ev(S, "NC 17 - See note 2", 11, table=T), ds.ev(S, "Pins GPIO6 to GPIO11 on the ESP32-D0WD-V3/ESP32-D0WDR2-V3 chip are connected to the SPI flash integrated on the module and are not led out.", 12, table=T)]))
    rules.append(rule("C-ESP32M-EN", "reset_pin", "fixed", "EN low switches the chip off; it must not be left floating", {"device": dev, "pin": no["EN"]}, {"active_level": "low"},
                      [ds.ev(S, "Low: Off; the chip shuts down", 11, table=T), ds.ev(S, "Note: Do not leave the pin floating.", 11, table=T)]))
    strap = [("IO0", "GPIO0 Pull-up 1", "high", "GPIO0"), ("IO2", "GPIO2 Pull-down 0", "low", "GPIO2"), ("IO12", "MTDI Pull-down 0", "low", "MTDI (GPIO12)"), ("IO15", "MTDO Pull-up 1", "high", "MTDO (GPIO15)"), ("IO5", "GPIO5 Pull-up 1", "high", "GPIO5")]
    for name, q, lvl, label in strap:
        rules.append(rule(f"C-ESP32M-STRAP-{name}", "strapping", "fixed", f"{label} is a strapping pin: its default level at reset is {lvl}", {"device": dev, "pin": no[name]},
                          {"sampled_at": "reset", "default_level": lvl, "external_drive_allowed": True},
                          [ds.ev(S, q, 13, table="Table 4: Default Configuration of Strapping Pins"), ds.ev(S, f"{name} {no[name]} I/O", table_page({"row": f"{name} {no[name]} I/O"}), table=T)]))
    rules.append(rule("C-ESP32M-BOOT-IO0", "boot_pin", "fixed", "IO0 and IO2 both low select the Joint Download Boot mode", {"device": dev, "pin": no["IO0"]},
                      {"boot_level": "low", "mode": "Joint Download Boot (SDIO or UART download); IO2 must be low as well"},
                      [ds.ev(S, "GPIO0 and GPIO2 control the boot mode after the reset is released.", 14, section="4.1 Chip Boot Mode Control"),
                       ds.ev(S, "Boot Mode GPIO0 GPIO2 SPI Boot Mode 1 Any value Joint Download Boot Mode 2 0 0", 14, table="Table 6: Chip Boot Mode Control")]))
    rules.append(rule("C-ESP32M-ELECTRICAL-VDD33", "electrical", "fixed", "Supply voltage 3.0 V to 3.6 V; the external supply must deliver at least 0.5 A", {"device": dev},
                      {"quantity": "VDD33 supply voltage and current", "condition": "recommended operating conditions (Table 14)", "min_v": 3.0, "typ_v": 3.3, "max_v": 3.6, "min_current_a": 0.5},
                      [ds.ev(S, "VDD33 Power supply voltage 3.0 3.3 3.6 V", 28, table="Table 14: Recommended Operating Conditions"), ds.ev(S, "Current delivered by external power supply 0.5", 28, table="Table 14: Recommended Operating Conditions")]))
    blocks = [
        {"id": "BLK-ESP32M-RESET", "title": "RC delay circuit on EN (usually 10 kΩ and 1 µF)", "kind": "reset_circuit", "modifiability": "configurable", "device": dev,
         "components": [{"role": "pull_up", "kind": "resistor", "min_count": 1, "value_ohm": 10000}, {"role": "delay", "kind": "capacitor", "min_count": 1, "min_value_nf": 1000}],
         "connects": [{"pin": no["EN"], "net_pattern": "*"}],
         "evidence": [sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")]},
        {"id": "BLK-ESP32M-BOOT", "title": "A way to pull IO0 low (button or jumper) to enter download mode", "kind": "boot_button", "modifiability": "configurable", "device": dev,
         "components": [{"role": "button", "kind": "switch", "min_count": 1}], "connects": [{"pin": no["IO0"], "net_pattern": "*"}],
         "evidence": [ds.ev(S, "Boot Mode GPIO0 GPIO2 SPI Boot Mode 1 Any value Joint Download Boot Mode 2 0 0", 14, table="Table 6: Chip Boot Mode Control", basis="derived")]},
    ]
    rules += [
        rule("C-ESP32M-NEED-RESET", "required_block", "configurable", "An RC delay circuit on EN is advised: usually 10 kΩ and 1 µF, adjusted to the supply", {"block": "BLK-ESP32M-RESET"},
             evidence=[sch.ev(G, "it is advised to add an RC delay circuit at the CHIP_PU pin.", section="Chip Power-up and Reset Timing"), sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")], severity="warning"),
        rule("C-ESP32M-NEED-BOOT", "required_block", "derived", "Download mode needs IO0 pulled low: a button, a jumper or a programmer's output", {"block": "BLK-ESP32M-BOOT"},
             evidence=[ds.ev(S, "Boot Mode GPIO0 GPIO2 SPI Boot Mode 1 Any value Joint Download Boot Mode 2 0 0", 14, table="Table 6: Chip Boot Mode Control", basis="derived")], derived_from=["C-ESP32M-BOOT-IO0"], severity="warning"),
        rule("C-ESP32M-EN-RC-VALUES", "required_block", "unknown", "The final values of the EN RC circuit", {"block": "BLK-ESP32M-RESET"},
             missing="The guideline says `usually R = 10 kΩ and C = 1 μF` and that the values are adjusted to the power-up and reset timing of the project. No document gives final values."),
        rule("C-ESP32M-ANT-KEEPOUT", "keepout", "unknown", "The antenna keep-out zone of ESP32-WROOM-32E: its size and position", None,
             evidence=[ds.ev(S, "The zone marked with dotted lines is the antenna keepout zone.", 10, figure="Figure 3: Pin Layout (Top View)")],
             missing="The datasheet says that a keep-out zone exists (the dotted zone of Figure 3) and points to the Hardware Design Guidelines for it on the base board; its dimensions are drawn in a figure only. No number was read for this pack: the keep-out is not checked."),
        rule("C-ESP32M-VARIANT-R2", "reserved_pin", "unknown", "Other variants (N4R2, N8R2, N16R2): IO16 is connected to the embedded PSRAM", {"device": dev}, {"pins": []},
             evidence=[ds.ev(S, "In module variants that have embedded QSPI PSRAM, i.e., that embed ESP32-D0WDR2-V3, IO16 is connected to the embedded PSRAM and can not be used for other functions.", 12, table=T)],
             missing="This pack is for ESP32-WROOM-32E-N4, which has no PSRAM. The datasheet says IO16 is not free on the R2 variants; those variants are not part of this pack (they would be another pack) and nothing is assumed for them."),
    ]
    up = [{"id": ESP32_CHIP, "version": VERSION, "kind": "chip", "canonical_sha256": digest(ESP32_CHIP), "inherit": {"devices": ["DEV-ESP32-D0WD-V3"], "rules": inheritable(ESP32_CHIP)}}]
    hcp = {
        "hcp_version": "1.1",
        "pack": pack_meta(ESP32_MOD, "module", "ESP32-WROOM-32E-N4: module-level constraints", REVIEW_NOTE, upstream=up),
        "sources": [source("DOC-ESP32-WROOM32E-DS", S, [dev]), source("DOC-ESP32-HDG-SCH", G, [dev]), source("DOC-ESP32-DS", "SRC-ESP32-DS", [dev, "DEV-ESP32-D0WD-V3"])],
        "devices": [{"id": dev, "vendor": "Espressif", "part_number": "ESP32-WROOM-32E-N4", "package": "module 18.0 x 25.5 x 3.1 mm, 38 pins", "pins": pins}],
        "peripheral_blocks": blocks, "rules": rules,
    }
    write_pack(ESP32_MOD, "module", "ESP32-WROOM-32E-N4", hcp, [(ESP32_CHIP, "chip")], ["DOC-ESP32-WROOM32E-DS", "DOC-ESP32-HDG-SCH", "DOC-ESP32-DS"], package="module 38 pins")


# ---- ESP32-C3 -----------------------------------------------------------------------------------------------------------------

C3_CHIP = "espressif/esp32-c3/esp32-c3/qfn32"
C3_MOD = "espressif/esp32-c3/esp32-c3-wroom-02/n4"


def chip_pin(d, name, hint_pages=None):
    """Number, page and quotation of a pin of a chip's pin table, found in the page text."""
    for pg in (hint_pages or range(1, len(pdfq.pages(d.path)) + 1)):
        text = pdfq.pages(d.path)[pg - 1]
        for pat, form in ((rf"(?<![A-Za-z0-9_]){name} (\d+) (I/O/T|I/O|I|P)(?= )", "{n} {no} {t}"), (rf"(?<![0-9]) (\d+) {name} (IO|Analog|Power)(?= )", "{no} {n} {t}")):
            m = re.search(pat, text)
            if m:
                no, t = m.group(1), m.group(2)
                return no, pg, form.format(n=name, no=no, t=t)
    sys.exit(f"pin {name} not found in the pin table of {d.id}")


def build_c3_chip():
    dev = "DEV-ESP32C3-QFN32"
    ds, sch, pcb = doc("DOC-ESP32C3-DS"), doc("DOC-ESP32C3-HDG-SCH"), doc("DOC-ESP32C3-HDG-PCB")
    S, G, P = "SRC-ESP32C3-DS", "SRC-ESP32C3-HDG-SCH", "SRC-ESP32C3-HDG-PCB"
    need = [("CHIP_EN", "reset"), ("GPIO2", "strapping"), ("GPIO8", "strapping"), ("GPIO9", "strapping"), ("GPIO18", "usb"), ("GPIO19", "usb")]
    pins, no = [], {}
    for name, cls in need:
        n, pg, q = chip_pin(ds, name, range(14, 20))
        no[name] = n
        pins.append({"number": n, "name": name, "class": cls, "evidence": [ds.ev(S, q, pg, table="Table 2-1. Pin Overview")]})
    T = "Table 3-1. Default Configuration of Strapping Pins"
    rules = [
        rule("C-C3C-STRAP-GPIO2", "strapping", "fixed", "GPIO2 is a strapping pin: floating by default (pull it up: it avoids glitches)", {"device": dev, "pin": no["GPIO2"]},
             {"sampled_at": "reset", "default_level": "floating", "external_drive_allowed": True}, [ds.ev(S, "GPIO2 Floating", 30, table=T)]),
        rule("C-C3C-STRAP-GPIO8", "strapping", "fixed", "GPIO8 is a strapping pin: floating by default", {"device": dev, "pin": no["GPIO8"]},
             {"sampled_at": "reset", "default_level": "floating", "external_drive_allowed": True}, [ds.ev(S, "GPIO8 Floating", 30, table=T)]),
        rule("C-C3C-STRAP-GPIO9", "strapping", "fixed", "GPIO9 is a strapping pin: weak pull-up (1) by default", {"device": dev, "pin": no["GPIO9"]},
             {"sampled_at": "reset", "default_level": "high", "external_drive_allowed": True}, [ds.ev(S, "GPIO9 Weak pull-up 1", 30, table=T)]),
        rule("C-C3C-BOOT-GPIO9", "boot_pin", "fixed", "GPIO8 high and GPIO9 low select the Joint Download Boot mode", {"device": dev, "pin": no["GPIO9"]},
             {"boot_level": "low", "mode": "Joint download boot (USB-Serial-JTAG or UART download); GPIO8 must be high"},
             [ds.ev(S, "GPIO2, GPIO8, and GPIO9 control the boot mode after the reset is released.", 31, section="3.1 Chip Boot Mode Control"),
              ds.ev(S, "SPI boot mode 1 Any value 1 Joint download boot mode 3 1 1 0", 31, table="Table 3-3. Chip Boot Mode Control")]),
        rule("C-C3C-RESET", "reset_pin", "fixed", "CHIP_EN low powers the chip off; it must not be left floating", {"device": dev, "pin": no["CHIP_EN"]}, {"active_level": "low"},
             [ds.ev(S, "Low: off, the chip powers off", 25, table="Table 2-1. Pin Overview") if ds.ok("Low: off, the chip powers off", 25) else ds.ev(S, "Do not leave the CHIP_EN pin floating", 25, table="Table 2-1. Pin Overview"),
              ds.ev(S, "Do not leave the CHIP_EN pin floating", 25, table="Table 2-1. Pin Overview")]),
        rule("C-C3C-ELECTRICAL-SUPPLY", "electrical", "fixed", "VDDA, VDD3P3 and VDD3P3_RTC take 3.0 V to 3.6 V (3.3 V typical); the cumulative input current is at least 0.5 A", {"device": dev},
             {"quantity": "VDDA, VDD3P3, VDD3P3_RTC input voltage and cumulative input current", "condition": "Table 5-2. Recommended Operating Conditions", "min_v": 3.0, "typ_v": 3.3, "max_v": 3.6, "min_current_a": 0.5},
             [ds.ev(S, "VDDA, VDD3P3, VDD3P3_RTC Recommended input voltage 3.0 3.3 3.6 V", 55, table="Table 5-2. Recommended Operating Conditions"),
              ds.ev(S, "IV DD Cumulative input current 0.5", 55, table="Table 5-2. Recommended Operating Conditions")]),
        rule("C-C3C-USB", "high_speed", "configurable", "USB differential pair: 90 Ω ±10%, in parallel and of equal length", None,
             {"nets": ["USB_D+", "USB_D-", "USB_DP", "USB_DM"], "differential": True, "impedance_ohm": 90, "impedance_tolerance_pct": 10},
             [pcb.ev(P, pcb.sent("Use differential pairs and route them in parallel at equal lengths.", 2), section="USB")]),
        rule("C-C3C-USB-SKEW", "high_speed", "unknown", "The length difference limit of the USB pair", None, {"nets": ["USB_D+", "USB_D-", "USB_DP", "USB_DM"], "differential": True, "skew_mm": None},
             missing="The guideline says `route them in parallel at equal lengths` and gives no number; no document read for this pack gives a skew limit in millimetres: no typical value is used."),
    ]
    blocks = [
        {"id": "BLK-C3C-RESET", "title": "RC delay circuit on CHIP_EN (usually 10 kΩ and 1 µF)", "kind": "reset_circuit", "modifiability": "configurable", "device": dev,
         "components": [{"role": "pull_up", "kind": "resistor", "min_count": 1, "value_ohm": 10000}, {"role": "delay", "kind": "capacitor", "min_count": 1, "min_value_nf": 1000}],
         "connects": [{"pin": no["CHIP_EN"], "net_pattern": "*"}],
         "evidence": [sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")]},
        {"id": "BLK-C3C-BOOT", "title": "A way to pull GPIO9 low (button or jumper) to enter download mode", "kind": "boot_button", "modifiability": "configurable", "device": dev,
         "components": [{"role": "button", "kind": "switch", "min_count": 1}], "connects": [{"pin": no["GPIO9"], "net_pattern": "*"}],
         "evidence": [ds.ev(S, "SPI boot mode 1 Any value 1 Joint download boot mode 3 1 1 0", 31, table="Table 3-3. Chip Boot Mode Control", basis="derived")]},
        {"id": "BLK-C3C-CRYSTAL", "title": "External 40 MHz crystal (compulsory)", "kind": "generic", "modifiability": "required", "device": dev,
         "components": [{"role": "crystal", "kind": "other", "min_count": 1}],
         "evidence": [sch.ev(G, "External crystal clock source (Compulsory)", section="Clock Source"), sch.ev(G, "The ESP32-C3 firmware only supports 40 MHz crystal.", section="External Crystal Clock Source (Compulsory)")]},
    ]
    rules += [
        rule("C-C3C-NEED-RESET", "required_block", "configurable", "An RC delay circuit on CHIP_EN is advised: usually 10 kΩ and 1 µF, adjusted to the supply", {"block": "BLK-C3C-RESET"}, severity="warning",
             evidence=[sch.ev(G, "it is advised to add an RC delay circuit at the CHIP_EN pin.", section="Chip Power-up and Reset Timing"), sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")]),
        rule("C-C3C-NEED-BOOT", "required_block", "derived", "Download mode needs GPIO9 pulled low (and GPIO8 high): a button, a jumper or a programmer's output", {"block": "BLK-C3C-BOOT"}, severity="warning", derived_from=["C-C3C-BOOT-GPIO9"],
             evidence=[ds.ev(S, "SPI boot mode 1 Any value 1 Joint download boot mode 3 1 1 0", 31, table="Table 3-3. Chip Boot Mode Control", basis="derived")]),
        rule("C-C3C-NEED-CRYSTAL", "required_block", "required", "The chip needs an external crystal, and the firmware supports 40 MHz only", {"block": "BLK-C3C-CRYSTAL"},
             evidence=[sch.ev(G, "External crystal clock source (Compulsory)", section="Clock Source"), sch.ev(G, "The ESP32-C3 firmware only supports 40 MHz crystal.", section="External Crystal Clock Source (Compulsory)")]),
        rule("C-C3C-EN-RC-VALUES", "required_block", "unknown", "The final values of the CHIP_EN RC circuit", {"block": "BLK-C3C-RESET"},
             missing="The guideline says `usually R = 10 kΩ and C = 1 μF` and that the values are adjusted to the power supply and the reset timing of the project. No document gives final values."),
    ]
    hcp = {
        "hcp_version": "1.1",
        "pack": pack_meta(C3_CHIP, "chip", "ESP32-C3 (QFN32): chip-level constraints", REVIEW_NOTE),
        "sources": [source("DOC-ESP32C3-DS", S, [dev]), source("DOC-ESP32C3-HDG-SCH", G, [dev]), source("DOC-ESP32C3-HDG-PCB", P, [dev])],
        "devices": [{"id": dev, "vendor": "Espressif", "part_number": "ESP32-C3", "package": "QFN32", "pins": pins}],
        "peripheral_blocks": blocks, "rules": rules,
    }
    write_pack(C3_CHIP, "chip", "ESP32-C3", hcp, [], ["DOC-ESP32C3-DS", "DOC-ESP32C3-HDG-SCH", "DOC-ESP32C3-HDG-PCB"], package="QFN32")


def build_c3_module():
    dev = "DEV-ESP32C3-WROOM02-N4"
    ds, sch, pcb = doc("DOC-ESP32C3-WROOM02-DS"), doc("DOC-ESP32C3-HDG-SCH"), doc("DOC-ESP32C3-HDG-PCB")
    S, G, P = "SRC-ESP32C3-WROOM02-DS", "SRC-ESP32C3-HDG-SCH", "SRC-ESP32C3-HDG-PCB"
    raw = module_pins(ds, [10, 11], "Table 3-1. Pin Definitions Name No. Type1 Function", "1 P: power supply", expect=19)
    strap = {"IO2", "IO8", "IO9"}

    def cls(p):
        n = p["name"]
        return "power" if n == "3V3" else "ground" if n == "GND" else "reset" if n == "EN" else "strapping" if n in strap else "usb" if n in ("IO18", "IO19") else "uart" if n in ("TXD", "RXD") else "gpio"

    def table_page(p):
        for pg in (10, 11):
            if ds.ok(p["row"], pg):
                return pg
        sys.exit(f"pin row not on page 10 or 11: {p['row']}")

    pins = [pin_entry(ds, S, p, cls(p), table_page, "Table 3-1. Pin Definitions") for p in raw]
    by = {}
    for p in raw:
        by.setdefault(p["name"], []).append(p["number"])
    no = {n: v[0] for n, v in by.items()}
    T = "Table 3-1. Pin Definitions"
    rules = []
    for gnd in by["GND"]:
        rules.append(rule(f"C-C3M-GND-{gnd}", "fixed_net", "fixed", f"Pin {gnd} is ground", {"device": dev}, {"pin": gnd, "net_pattern": "GND*"}, [ds.ev(S, "GND 9,19 P Ground", 11, table=T)]))
    rules.append(rule("C-C3M-EN", "reset_pin", "fixed", "EN low powers the chip off; it must not be left floating", {"device": dev, "pin": no["EN"]}, {"active_level": "low"},
                      [ds.ev(S, "Low: off, the chip powers off.", 11, table=T), ds.ev(S, "Note: Do not leave the EN pin floating.", 11, table=T)]))
    for name, q, lvl, gp in (("IO2", "GPIO2 Floating", "floating", "GPIO2"), ("IO8", "GPIO8 Floating", "floating", "GPIO8"), ("IO9", "GPIO9 Weak pull-up 1", "high", "GPIO9")):
        rules.append(rule(f"C-C3M-STRAP-{name}", "strapping", "fixed", f"{gp} is a strapping pin: default level {lvl}", {"device": dev, "pin": no[name]},
                          {"sampled_at": "reset", "default_level": lvl, "external_drive_allowed": True},
                          [ds.ev(S, q, 12, table="Table 4-1. Default Configuration of Strapping Pins"), ds.ev(S, f"{name} {no[name]} I/O/T", table_page({"row": f"{name} {no[name]} I/O/T"}), table=T)]))
    rules.append(rule("C-C3M-BOOT-IO9", "boot_pin", "fixed", "IO8 high and IO9 low select the Joint Download Boot mode", {"device": dev, "pin": no["IO9"]},
                      {"boot_level": "low", "mode": "Joint download boot (USB-Serial-JTAG or UART download); IO8 must be high"},
                      [ds.ev(S, "GPIO2, GPIO8, and GPIO9 control the boot mode after the reset is released.", 13, section="4.1 Chip Boot Mode Control"),
                       ds.ev(S, "SPI boot mode 1 Any value 1 Joint download boot mode 3 1 1 0", 13, table="Table 4-3. Chip Boot Mode Control")]))
    rules.append(rule("C-C3M-ELECTRICAL-VDD33", "electrical", "fixed", "Supply voltage 3.0 V to 3.6 V; the external supply must deliver at least 0.5 A", {"device": dev},
                      {"quantity": "VDD33 supply voltage and current", "condition": "Table 6-2. Recommended Operating Conditions", "min_v": 3.0, "typ_v": 3.3, "max_v": 3.6, "min_current_a": 0.5},
                      [ds.ev(S, "VDD33 Power supply voltage 3.0 3.3 3.6 V", 21, table="Table 6-2. Recommended Operating Conditions"), ds.ev(S, "Current delivered by external power supply 0.5", 21, table="Table 6-2. Recommended Operating Conditions")]))
    rules.append(rule("C-C3M-USB", "high_speed", "configurable", "USB differential pair on IO18 (D-) and IO19 (D+): 90 Ω ±10%, in parallel and of equal length", None,
                      {"nets": ["USB_D+", "USB_D-", "USB_DP", "USB_DM"], "differential": True, "impedance_ohm": 90, "impedance_tolerance_pct": 10},
                      [pcb.ev(P, pcb.sent("Use differential pairs and route them in parallel at equal lengths.", 2), section="USB"), ds.ev(S, "IO18 13 I/O/T", 11, table=T)]))
    rules.append(rule("C-C3M-USB-SKEW", "high_speed", "unknown", "The length difference limit of the USB pair", None, {"nets": ["USB_D+", "USB_D-", "USB_DP", "USB_DM"], "differential": True, "skew_mm": None},
                      missing="The guideline says `route them in parallel at equal lengths` and gives no number: no skew limit is used."))
    blocks = [
        {"id": "BLK-C3M-RESET", "title": "RC delay circuit on EN (usually 10 kΩ and 1 µF)", "kind": "reset_circuit", "modifiability": "configurable", "device": dev,
         "components": [{"role": "pull_up", "kind": "resistor", "min_count": 1, "value_ohm": 10000}, {"role": "delay", "kind": "capacitor", "min_count": 1, "min_value_nf": 1000}],
         "connects": [{"pin": no["EN"], "net_pattern": "*"}],
         "evidence": [sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")]},
        {"id": "BLK-C3M-BOOT", "title": "A way to pull IO9 low (button or jumper) to enter download mode", "kind": "boot_button", "modifiability": "configurable", "device": dev,
         "components": [{"role": "button", "kind": "switch", "min_count": 1}], "connects": [{"pin": no["IO9"], "net_pattern": "*"}],
         "evidence": [ds.ev(S, "SPI boot mode 1 Any value 1 Joint download boot mode 3 1 1 0", 13, table="Table 4-3. Chip Boot Mode Control", basis="derived")]},
    ]
    rules += [
        rule("C-C3M-NEED-RESET", "required_block", "configurable", "An RC delay circuit on EN is advised: usually 10 kΩ and 1 µF, adjusted to the supply", {"block": "BLK-C3M-RESET"}, severity="warning",
             evidence=[sch.ev(G, "it is advised to add an RC delay circuit at the CHIP_EN pin.", section="Chip Power-up and Reset Timing"), sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")]),
        rule("C-C3M-NEED-BOOT", "required_block", "derived", "Download mode needs IO9 pulled low (and IO8 high): a button, a jumper or a programmer's output", {"block": "BLK-C3M-BOOT"}, severity="warning", derived_from=["C-C3M-BOOT-IO9"],
             evidence=[ds.ev(S, "SPI boot mode 1 Any value 1 Joint download boot mode 3 1 1 0", 13, table="Table 4-3. Chip Boot Mode Control", basis="derived")]),
        rule("C-C3M-EN-RC-VALUES", "required_block", "unknown", "The final values of the EN RC circuit", {"block": "BLK-C3M-RESET"},
             missing="The guideline says `usually R = 10 kΩ and C = 1 μF` and that the values are adjusted to the power supply and the reset timing of the project. No document gives final values."),
        rule("C-C3M-ANT-KEEPOUT", "keepout", "unknown", "The antenna keep-out zone of ESP32-C3-WROOM-02: its size and position", None,
             evidence=[ds.ev(S, "The zone marked with dotted lines is the antenna keepout zone.", 10, figure="Pin Layout (Top View)")],
             missing="The datasheet says that a keep-out zone exists and points to the Hardware Design Guidelines for it on the base board; its dimensions are drawn in a figure only. No number was read for this pack: the keep-out is not checked."),
    ]
    up = [{"id": C3_CHIP, "version": VERSION, "kind": "chip", "canonical_sha256": digest(C3_CHIP),
           "inherit": {"devices": ["DEV-ESP32C3-QFN32"], "rules": inheritable(C3_CHIP)}}]
    hcp = {
        "hcp_version": "1.1",
        "pack": pack_meta(C3_MOD, "module", "ESP32-C3-WROOM-02-N4: module-level constraints", REVIEW_NOTE, upstream=up),
        "sources": [source("DOC-ESP32C3-WROOM02-DS", S, [dev]), source("DOC-ESP32C3-HDG-SCH", G, [dev]), source("DOC-ESP32C3-HDG-PCB", P, [dev]), source("DOC-ESP32C3-DS", "SRC-ESP32C3-DS", [dev, "DEV-ESP32C3-QFN32"])],
        "devices": [{"id": dev, "vendor": "Espressif", "part_number": "ESP32-C3-WROOM-02-N4", "package": "module 18.0 x 20.0 x 3.2 mm, 19 pins", "pins": pins}],
        "peripheral_blocks": blocks, "rules": rules,
    }
    write_pack(C3_MOD, "module", "ESP32-C3-WROOM-02-N4", hcp, [(C3_CHIP, "chip")], ["DOC-ESP32C3-WROOM02-DS", "DOC-ESP32C3-HDG-SCH", "DOC-ESP32C3-HDG-PCB", "DOC-ESP32C3-DS"], package="module 19 pins")


# ---- ESP32-S3 ----------------------------------------------------------------------------------------------------------------

S3_CHIP = "espressif/esp32-s3/esp32-s3/qfn56"
S3_MOD = "espressif/esp32-s3/esp32-s3-wroom-1/n8r8"
DATA = HERE / "data"


def build_s3_chip():
    dev = "DEV-ESP32S3"
    ds, sch, pcb = doc("DOC-ESP32S3-DS"), doc("DOC-ESP32S3-HDG-SCH"), doc("DOC-ESP32S3-HDG-PCB")
    S, G, P = "SRC-ESP32S3-DS", "SRC-ESP32S3-HDG-SCH", "SRC-ESP32S3-HDG-PCB"
    cls = {"CHIP_PU": "reset", "GPIO0": "strapping", "GPIO3": "strapping", "GPIO45": "strapping", "GPIO46": "strapping", "GPIO19": "usb", "GPIO20": "usb"}
    pins, no = [], {}
    for name, c in cls.items():
        n, pg, q = chip_pin(ds, name, range(14, 20))
        no[name] = n
        pins.append({"number": n, "name": name, "class": c, "evidence": [ds.ev(S, q, pg, table="Table 2-1. Pin Overview")]})
    T = "Table 3-1. Default Configuration of Strapping Pins"
    rules = [
        rule("C-S3C-STRAP-GPIO0", "strapping", "fixed", "GPIO0 is a strapping pin: weak pull-up (1) by default", {"device": dev, "pin": no["GPIO0"]}, {"sampled_at": "reset", "default_level": "high", "external_drive_allowed": True}, [ds.ev(S, "GPIO0 Weak pull-up 1", 32, table=T)]),
        rule("C-S3C-STRAP-GPIO46", "strapping", "fixed", "GPIO46 is a strapping pin: weak pull-down (0) by default", {"device": dev, "pin": no["GPIO46"]}, {"sampled_at": "reset", "default_level": "low", "external_drive_allowed": True}, [ds.ev(S, "GPIO46 Weak pull-down 0", 32, table=T)]),
        rule("C-S3C-STRAP-GPIO45", "strapping", "fixed", "GPIO45 is a strapping pin: weak pull-down (0) by default", {"device": dev, "pin": no["GPIO45"]}, {"sampled_at": "reset", "default_level": "low", "external_drive_allowed": True}, [ds.ev(S, "GPIO45 Weak pull-down 0", 32, table=T)]),
        rule("C-S3C-STRAP-GPIO3", "strapping", "fixed", "GPIO3 is a strapping pin without internal pull resistors", {"device": dev, "pin": no["GPIO3"]}, {"sampled_at": "reset", "default_level": "floating", "external_drive_allowed": True},
             [ds.ev(S, "This pin does not have any internal pull resistors and the strapping value must be controlled by the external circuit that cannot be in a high impedance state.", 34, section="3.3 JTAG Signal Source Control")],
             applies_when=["GPIO3 is used to select the JTAG signal source"]),
        rule("C-S3C-BOOT-GPIO0", "boot_pin", "fixed", "GPIO0 and GPIO46 both low select the Joint Download Boot mode", {"device": dev, "pin": no["GPIO0"]}, {"boot_level": "low", "mode": "Joint Download Boot (UART or USB download); GPIO46 low as well"},
             [ds.ev(S, "GPIO0 and GPIO46 control the boot mode after the reset is released.", 33, section="3.1 Chip Boot Mode Control"), ds.ev(S, "Boot Mode GPIO0 GPIO46 SPI boot mode 1 Any value Joint download boot mode 2 0 0", 33, table="Table 3-3. Chip Boot Mode Control")]),
        rule("C-S3C-RESET", "reset_pin", "fixed", "CHIP_PU low powers the chip off; it must not be left floating", {"device": dev, "pin": no["CHIP_PU"]}, {"active_level": "low"}, [ds.ev(S, "Do not leave the CHIP_PU pin floating.", 28, table="Table 2-9. Analog Pins") if ds.ok("Do not leave the CHIP_PU pin floating.", 28) else ds.ev(S, "Do not leave the CHIP_PU pin floating.", 28)]),
        rule("C-S3C-ELECTRICAL-SUPPLY", "electrical", "fixed", "VDDA and VDD3P3 take 3.0 V to 3.6 V (3.3 V typical); the cumulative input current is at least 0.5 A", {"device": dev},
             {"quantity": "VDDA, VDD3P3 input voltage and cumulative input current", "condition": "Table 5-2. Recommended Operating Conditions", "min_v": 3.0, "typ_v": 3.3, "max_v": 3.6, "min_current_a": 0.5},
             [ds.ev(S, "VDDA, VDD3P3 Recommended input voltage 3.0 3.3 3.6 V", 64, table="Table 5-2. Recommended Operating Conditions"), ds.ev(S, "Cumulative input current 0.5", 64, table="Table 5-2. Recommended Operating Conditions")]),
        rule("C-S3C-USB", "high_speed", "configurable", "USB differential pair on GPIO19 (D-) and GPIO20 (D+): 90 Ω ±10%, in parallel and of equal length", None,
             {"nets": ["USB_D+", "USB_D-", "USB_DP", "USB_DM"], "differential": True, "impedance_ohm": 90, "impedance_tolerance_pct": 10},
             [pcb.ev(P, pcb.sent("Use differential pairs and route them in parallel at equal lengths.", 2), section="USB"), sch.ev(G, "GPIO19 and GPIO20 can be used as D- and D+ of USB respectively.", section="USB")]),
        rule("C-S3C-USB-SKEW", "high_speed", "unknown", "The length difference limit of the USB pair", None, {"nets": ["USB_D+", "USB_D-", "USB_DP", "USB_DM"], "differential": True, "skew_mm": None},
             missing="The guideline says `route them in parallel at equal lengths` and gives no number: no skew limit is used."),
    ]
    blocks = [
        {"id": "BLK-S3C-RESET", "title": "RC delay circuit on CHIP_PU (usually 10 kΩ and 1 µF)", "kind": "reset_circuit", "modifiability": "configurable", "device": dev,
         "components": [{"role": "pull_up", "kind": "resistor", "min_count": 1, "value_ohm": 10000}, {"role": "delay", "kind": "capacitor", "min_count": 1, "min_value_nf": 1000}],
         "connects": [{"pin": no["CHIP_PU"], "net_pattern": "*"}], "evidence": [sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")]},
        {"id": "BLK-S3C-BOOT", "title": "A way to pull GPIO0 low (button or jumper) to enter download mode", "kind": "boot_button", "modifiability": "configurable", "device": dev,
         "components": [{"role": "button", "kind": "switch", "min_count": 1}], "connects": [{"pin": no["GPIO0"], "net_pattern": "*"}],
         "evidence": [ds.ev(S, "Boot Mode GPIO0 GPIO46 SPI boot mode 1 Any value Joint download boot mode 2 0 0", 33, table="Table 3-3. Chip Boot Mode Control", basis="derived")]},
        {"id": "BLK-S3C-CRYSTAL", "title": "External 40 MHz crystal (compulsory)", "kind": "generic", "modifiability": "required", "device": dev, "components": [{"role": "crystal", "kind": "other", "min_count": 1}],
         "evidence": [sch.ev(G, "External Crystal Clock Source (Compulsory)", section="Clock Source"), sch.ev(G, "The ESP32-S3 firmware only supports 40 MHz crystal.", section="External Crystal Clock Source (Compulsory)")]},
    ]
    rules += [
        rule("C-S3C-NEED-RESET", "required_block", "configurable", "An RC delay circuit on CHIP_PU is advised: usually 10 kΩ and 1 µF, adjusted to the supply", {"block": "BLK-S3C-RESET"}, severity="warning",
             evidence=[sch.ev(G, "it is advised to add an RC delay circuit at the CHIP_PU pin.", section="Chip Power-up and Reset Timing"), sch.ev(G, sch.sent("The recommended setting for the RC delay circuit"), section="Chip Power-up and Reset Timing")]),
        rule("C-S3C-NEED-BOOT", "required_block", "derived", "Download mode needs GPIO0 pulled low: a button, a jumper or a programmer's output", {"block": "BLK-S3C-BOOT"}, severity="warning", derived_from=["C-S3C-BOOT-GPIO0"],
             evidence=[ds.ev(S, "Boot Mode GPIO0 GPIO46 SPI boot mode 1 Any value Joint download boot mode 2 0 0", 33, table="Table 3-3. Chip Boot Mode Control", basis="derived")]),
        rule("C-S3C-NEED-CRYSTAL", "required_block", "required", "The chip needs an external crystal, and the firmware supports 40 MHz only", {"block": "BLK-S3C-CRYSTAL"},
             evidence=[sch.ev(G, "External Crystal Clock Source (Compulsory)", section="Clock Source"), sch.ev(G, "The ESP32-S3 firmware only supports 40 MHz crystal.", section="External Crystal Clock Source (Compulsory)")]),
        rule("C-S3C-EN-RC-VALUES", "required_block", "unknown", "The final values of the CHIP_PU RC circuit", {"block": "BLK-S3C-RESET"},
             missing="The guideline says `usually R = 10 kΩ and C = 1 μF` and that the values are adjusted to the power supply and the reset timing of the project. No document gives final values."),
    ]
    hcp = {
        "hcp_version": "1.1",
        "pack": pack_meta(S3_CHIP, "chip", "ESP32-S3 (QFN56): chip-level constraints", REVIEW_NOTE),
        "sources": [source("DOC-ESP32S3-DS", S, [dev]), source("DOC-ESP32S3-HDG-SCH", G, [dev]), source("DOC-ESP32S3-HDG-PCB", P, [dev])],
        "devices": [{"id": dev, "vendor": "Espressif", "part_number": "ESP32-S3", "package": "QFN56", "pins": pins}],
        "peripheral_blocks": blocks, "rules": rules,
    }
    write_pack(S3_CHIP, "chip", "ESP32-S3", hcp, [], ["DOC-ESP32S3-DS", "DOC-ESP32S3-HDG-SCH", "DOC-ESP32S3-HDG-PCB"], package="QFN56")


S3_MOD16 = "espressif/esp32-s3/esp32-s3-wroom-1/n16r8"


def s3_module_data(variant):
    """The module-level facts of one WROOM-1 variant. The pin table, the boot configuration, the supply and the circuits are the same for all of
    them (one datasheet); what differs is the device id, the part number and the ordering-table row that proves the octal PSRAM (IO35 to IO37)."""
    text = (DATA / "esp32-s3-wroom-1-n8r8.module.yaml").read_text(encoding="utf-8")
    if variant == "n16r8":
        text = text.replace("ESP32-S3-WROOM-1-N8R8 8 MB (Quad SPI) 8 MB (Octal SPI)", "ESP32-S3-WROOM-1-N16R8 16 MB (Quad SPI) 8 MB (Octal SPI)")
        text = text.replace("DEV-ESP32S3-WROOM1-N8R8", "DEV-ESP32S3-WROOM1-N16R8").replace("ESP32-S3-WROOM-1-N8R8", "ESP32-S3-WROOM-1-N16R8")
    return yaml.safe_load(text)


def build_s3_module(variant="n8r8"):
    data = s3_module_data(variant)
    pid, part = (S3_MOD, "ESP32-S3-WROOM-1-N8R8") if variant == "n8r8" else (S3_MOD16, "ESP32-S3-WROOM-1-N16R8")
    dev = data["devices"][0]["id"]
    # the facts come from the file above; each quotation is looked up again on its page before anything is written
    docs_by_src = {"SRC-ESP32S3-WROOM1-DS": "DOC-ESP32S3-WROOM1-DS", "SRC-ESP32S3-DS": "DOC-ESP32S3-DS", "SRC-ESP32S3-HDG-SCH": "DOC-ESP32S3-HDG-SCH", "SRC-ESP32S3-HDG-PCB": "DOC-ESP32S3-HDG-PCB"}
    bad = []

    def recheck(obj):
        if isinstance(obj, dict):
            if "quote" in obj and "source_id" in obj and obj["basis"] != "pending":
                d = doc(docs_by_src[obj["source_id"]])
                if not d.ok(obj["quote"], obj.get("locator", {}).get("page")):
                    bad.append((obj["source_id"], obj["quote"][:90]))
            for v in obj.values():
                recheck(v)
        elif isinstance(obj, list):
            for v in obj:
                recheck(v)
    recheck(data)
    if variant != "n8r8" and bad:
        sys.exit("a quotation of the %s module is not found on its page: %s" % (variant, bad))
    for b in bad:
        print("  note: not found by the builder's normaliser:", *b, file=sys.stderr)
    up = [{"id": S3_CHIP, "version": VERSION, "kind": "chip", "canonical_sha256": digest(S3_CHIP), "inherit": {"devices": ["DEV-ESP32S3"], "rules": inheritable(S3_CHIP)}}]
    hcp = {"hcp_version": "1.1", "pack": pack_meta(pid, "module", part + " 模组级约束包", "草案，尚无人工复核。天线禁区的尺寸来自指南中的一张图（按图上的标注读取，待人工确认）；其余未在资料中给出的参数保持 unknown。", language="zh-CN", upstream=up)}
    hcp.update(data)
    write_pack(pid, "module", part, hcp, [(S3_CHIP, "chip")], ["DOC-ESP32S3-WROOM1-DS", "DOC-ESP32S3-DS", "DOC-ESP32S3-HDG-SCH", "DOC-ESP32S3-HDG-PCB"], package="module 41 pins")


def build_s3_module16():
    build_s3_module("n16r8")


def build_s3_example():
    """The first example pack (#16): the carrier overlay + the module pack, merged. pack.yaml is generated, never edited."""
    ex = ROOT / "esp32-s3-wroom-1-n8r8"
    text = (ex / "overlay.template.yaml").read_text(encoding="utf-8").replace("@@MODULE_DIGEST@@", digest(S3_MOD))
    tmp = ex / ".overlay.yaml"
    tmp.write_text(text, encoding="utf-8")
    flat = ex / ".resolved.yaml"
    out = subprocess.run([tool("fastroute"), "constraints", "resolve", str(tmp), "--upstream", str(pack_dir(S3_MOD) / "hcp.yaml"), "-o", str(flat), "--force"], capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit("resolve failed: " + out.stderr[-600:])
    body = flat.read_text(encoding="utf-8").split("\n", 1)[1]
    (ex / "pack.yaml").write_text("# GENERATED by hardware-packs/tools/build_espressif.py --only s3-example: the carrier overlay (overlay.template.yaml) merged with the module pack\n# espressif/esp32-s3/esp32-s3-wroom-1/n8r8 (and it with the chip pack). Edit those, not this file.\n" + body, encoding="utf-8")
    tmp.unlink()
    flat.unlink()


BUILDERS = {"esp32-chip": build_esp32_chip, "esp32-module": build_esp32_module, "c3-chip": build_c3_chip, "c3-module": build_c3_module, "s3-chip": build_s3_chip, "s3-module": build_s3_module, "s3-module16": build_s3_module16, "s3-example": build_s3_example}

if __name__ == "__main__":
    only = sys.argv[sys.argv.index("--only") + 1].split(",") if "--only" in sys.argv else list(BUILDERS)
    for k in only:
        print("building", k)
        BUILDERS[k]()
    print("done")
