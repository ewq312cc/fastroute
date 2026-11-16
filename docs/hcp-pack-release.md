# Releasing hardware constraint packs with the software

How packs for chips, modules and boards are laid out, versioned, locked, shipped with a fastroute / `hcp-manager` release, and installed
offline. This is the distribution mechanism only; the rules inside a pack are written and reviewed as described in [HCP.md](HCP.md). The
application that does all of this is [`hcp-manager`](HCP_MANAGER.md).

## Layout

```
hardware-packs/
  index.yaml                 generated: the catalogue and lock of every pack in the tree (committed)
  schemas/pack-manifest.schema.json
  tools/build-release.sh
  <vendor>/<family>/<device>/<variant>/<pack-version>/
      pack.yaml              the manifest (below)
      hcp.yaml               the HCP document: facts and evidence
      sources.yaml           the inventory entries of the documents the facts come from (version, digest, URL)
      reports/...            readable reports (optional)
      documents/...          original documents, only if their licence allows redistribution (optional)
```

The identity is **vendor / family / device / variant** and nothing in the tooling is specific to a vendor: a new vendor, series, device or
variant is a new directory, not a code change. The pack id is those four joined by `/`, the directory is `<id>/<pack-version>`, and the scanner
refuses a pack that is anywhere else (`path_mismatch`). Directories without a manifest (the older single-pack layout of
`hardware-packs/espressif/esp32-s3-wroom-1-n8r8/`) are not part of a release.

### Three kinds of pack

| Kind | States | May depend on |
|---|---|---|
| `chip` | what the chip requires | nothing |
| `module` | what the module adds or changes: integrated flash / PSRAM, antenna, pins brought out, mechanical limits | chip packs |
| `board` | what the board implements and exposes | module or chip packs |

A dependency is declared twice, and the two have to agree (`upstream_mismatch`): in the manifest, and as `pack.upstream` in `hcp.yaml`, which also
says **what** is inherited (named devices, rules, blocks, regions) and pins the upstream by the canonical digest of its HCP document. Nothing is
inherited that is not listed; a rule that replaces an inherited one (`overrides`) needs evidence from a source of the pack itself; a contradiction
without an override is an error (`inherit_conflict`). Details and the diagnostic codes: `skills/hardware-constraint-design/references/hcp-format.md`.
`hcp-manager packs resolve` (and `fastroute constraints resolve`) turn such a pack into the one flat pack that `check`, `compile` and `validate` read.

### Versions

Three versions are independent and recorded separately:

* the **pack version** (`pack.yaml: version`, semantic version, also in `hcp.yaml`): changes when the facts change;
* the **software version** (`release.yaml: software.version`): changes when fastroute / `hcp-manager` does. A release does not take "the latest" of
  anything: it lists each pack at an exact version and digest;
* the **format versions**: `hcp_schema_version` (the HCP document, `1.x`), `ir_version` (the Constraint IR the pack compiles to) and
  `manifest_version`. A pack declares which it was written for, plus `min_app_version` and optionally `max_app_version`.

A dependency is an **exact version** pinned by digest. There are no version ranges between packs: a module is reviewed against one chip pack, and
a different chip pack version is a new review. (The compatibility ranges that exist are the application's: `min_app_version` / `max_app_version`.)
When a dependency changes, the packs that inherit from it fail until they are reviewed and sealed again (`dependency_digest_mismatch`).

## The manifest (`pack.yaml`)

Schema: [`hardware-packs/schemas/pack-manifest.schema.json`](../hardware-packs/schemas/pack-manifest.schema.json). You write the first part;
`hcp-manager release seal <pack dir>` computes the rest.

| Field | Who | Meaning |
|---|---|---|
| `manifest_version`, `id`, `kind`, `identity`, `version`, `content`, `review`, `compatibility`, `dependencies` (id, kind, version), `license` | author | what the pack is, what it needs, what may be shipped |
| `files` | `seal` | path and SHA-256 of `hcp.yaml`, `sources.yaml`, every file in `reports/` and `documents/`. A file that is not listed is an error (`unlisted_file`) |
| `hcp_canonical_sha256` | `seal` | digest of the canonical JSON of `hcp.yaml`: what packs that inherit from this one pin; formatting does not change it |
| `source_digests` | `seal` | per cited document: version, SHA-256, URL and whether the original may be redistributed, taken from `sources.yaml` and `license` |
| `dependencies[].canonical_sha256` | `seal` | copied from `pack.upstream` of `hcp.yaml` |

`content: metadata_only` registers an identity and carries **no** pins, rules, blocks or regions (`metadata_only_with_rules`): that is how STC,
STM32 or any other vendor is made known to the release without writing a single unverified hardware fact. The fixtures of the tests do exactly this
(`apps/hcp-manager/tests/release.rs`); they use placeholder digests and say so.

### Licence of the originals

Every cited document has `redistribution: allowed` or `reference_only` in `license.documents` (sealing fails without a decision). With
`reference_only` the pack keeps the URL, the version, the SHA-256, the page / section locators and short quotations, and **nothing of the original**: a
file of such a document in the pack is `license_forbids_bundling`, found by its listed path and also by its digest under any other name. Only a document
that is `allowed` may be listed under `files.documents` (the pack directory then holds it as `documents/<DOCUMENT-ID>.<ext>`); it is installed with the
pack. The cache of the workspace is not seeded from it: `hcp-manager sources sync` or the inventory still decides where the cached copy comes from.

## Making a release

```sh
hcp-manager release seal <pack dir>           # digests of the files, canonical digest, locked sources; after every change of the pack
hcp-manager release index --packs hardware-packs [--check]
hcp-manager release build --packs hardware-packs --out dist/hcp-packs
hardware-packs/tools/build-release.sh [--regenerate-index]    # the three steps and a verification
```

* `release index` writes `hardware-packs/index.yaml`, the catalogue of the tree: id, kind, version, the digests of the manifest and of the HCP document (file and canonical),
  dependencies and compatibility. It is **committed**; `--check` fails when it is not the current catalogue, and `release build` refuses to build from
  a tree its lock does not describe. A software release therefore ships exactly the packs and digests that were reviewed in the diff of `index.yaml`.
* `release build` writes `dist/hcp-packs/`: `release.yaml` (the packs of this software release, each with archive and digests), `SHA256SUMS` (compatible
  with `sha256sum -c`; covers `release.yaml` and every archive) and `archives/<id>/v<version>.tar.gz`, one per pack.
* **Reproducible**: no clock, no host data. The archive is a `tar.gz` with sorted paths, fixed times, owners and modes and no gzip name or time, so the same
  files give the same bytes. (The compressed bytes depend on the compression library, so "the same bytes" holds for one `Cargo.lock`; the digests of the
  *content* — manifest, HCP document, canonical digest — do not depend on it.) Nothing is fetched while building; the locked documents are not needed.
* The tree and the set are verified first: manifests against the schema, every digest, identity and directory, HCP identity and version, the locked sources
  against `sources.yaml`, compatibility, licence, unique ids, dependencies present with the pinned digest and kind, no cycles, and the merged HCP document of
  every pack valid. Anything wrong stops the build and is listed with its code.

`dist/` is a build output; ship it beside the program (`<prefix>/share/hcp-packs/`), or point to it with `HCP_PACKS_DIR` / `--release DIR`. The packs are not
compiled into the binary: a release is a directory that can be listed, verified and installed from without a network.

## Installing into a workspace

```sh
hcp-manager packs builtin [--release DIR] list                 # the packs of the release; installed / modified / not installed
hcp-manager packs builtin [--release DIR] show <id>[@version]
hcp-manager packs builtin [--release DIR] verify              # checksums, every archive, every pack, the set; offline
hcp-manager packs builtin [--release DIR] install <id>[@version] [--force]
hcp-manager packs resolve <id>[@version] [-o flat.yaml]       # a pack that inherits, merged with its exact upstream packs
```

* The release directory is only **read**. `install` verifies the release first, resolves the exact dependencies (dependencies first, nothing similar, nothing
  newer), checks them together, and only then writes `packs/<id>/<version>/` of the workspace (the files of the pack and an `INSTALL.json` receipt with the
  digests as installed). The documents of the packs are added to the inventory; a document that is already there with another version or digest stops the
  install (the inventory is not changed by an install).
* **A local change is never overwritten silently.** A copy whose files no longer match its receipt is `modified`; installing again (for example after a
  software upgrade brought a new archive of the same version) refuses with exit 4. `--force` moves the local copy to `backup/packs/<id>/<version>-<digest>/`
  and installs the release's. The same pack installed twice is a no-op. A different version is a different directory: versions live side by side.
* Project-level changes are made to the installed copy (or a copy of it under a new version); nothing is ever written back to the release.
* Failures are deterministic and say which pack and why (`missing_dependency`, `dependency_digest_mismatch`, `archive_digest_mismatch`, `sums_mismatch`,
  `incompatible_ir_version`, `kind_not_allowed`, `license_forbids_bundling`, ...); the list of codes is in the output of `verify --json`.

The vector index is not part of a pack or of a release and is not needed for any of this: installing, resolving, compiling and validating read files only,
and deleting `derived/` (the index) changes none of them (tested). After installing, `hcp-manager index build` makes the index from the documents.

## Adding a vendor, family, device or variant

1. Pick the identity: `<vendor>/<family>/<device>/<variant>` (lowercase letters, digits, `.`, `_`, `-`). Create `<id>/<pack-version>/`.
2. Lock the documents: add them to the inventory (`hcp-manager sources add/sync`), copy the entries into the pack's `sources.yaml` (version, digest, URL).
3. Write `hcp.yaml` (`hcp_version: "1.1"`, `pack.kind`, the sources, devices, rules with evidence) as described in [HCP.md](HCP.md); for a module or board
   list what it inherits in `pack.upstream`. Or, to only register the identity, `pack.content: metadata_only` and no facts.
4. Write the authored part of `pack.yaml` (see a fixture in `apps/hcp-manager/tests/release.rs`): identity, kind, version, `content`, `compatibility`,
   dependencies, and for each document `license.documents`.
5. `hcp-manager release seal <dir>`, then `hardware-packs/tools/build-release.sh --regenerate-index` and review the diff of `index.yaml`.
6. Evidence check against the locked documents: `hcp-manager evidence verify` on the installed pack.

### Review checklist

* Every fact has evidence from the **right** document (exact model and revision; a similar model's document is never evidence), at the right level
  (official manual and errata, then guides and reference designs, then dev kits, then open boards with a version, then community).
* A module pack contains only module-level facts; nothing of the module (integrated flash, antenna, pins brought out) is written into a chip pack. Every
  override has evidence of the module's own documents. A chip rule that a module does not list is not inherited.
* Unknown stays `unknown` (with what is missing), disagreement is `conflict`; no value from a similar part or from memory.
* The manifest says what may be shipped (`license.documents`), and nothing of a `reference_only` document is in the directory.
* `index.yaml` changed exactly by the packs of this change; versions were raised where facts changed; dependants were reviewed and sealed again.
* `hcp-manager packs builtin verify` passes on the built release, in an environment without a network and without an index.

## Compatibility policy

* The manifest, the release format and the HCP document change only **additively** within a major version; a program refuses a pack for a newer minor
  version than it knows (`incompatible_hcp_schema`) or another IR version (`incompatible_ir_version`) instead of guessing.
* A software release that raises the IR version rebuilds the release; packs for the old IR version are refused, not converted.
* `min_app_version` / `max_app_version` are checked at install and verify time against the running `hcp-manager`.

## Not covered

* **Signatures.** Integrity is the manifest plus SHA-256 (`SHA256SUMS`, `release.yaml`, digests inside the manifests). Whoever can replace `release.yaml`,
  `SHA256SUMS` and an archive together is not detected; signing a release is left as a later extension.
* **Packs compiled into the binary.** A release is a directory beside the program.
* **Automatic upgrade** of installed packs, and ranges between packs: an upgrade is an explicit `install` of a new version.
* **Real packs beyond the six.** The release ships with six drafts for Espressif's ESP32, ESP32-S3 and ESP32-C3 (a chip and a module each; what they cover and what is
  unknown is in `hardware-packs/espressif/coverage.yaml` and `README.md`), plus the fictional packs of the tests. No STC or STM32 pack of real content exists;
  their metadata-only fixtures show that a pack can be listed without its documents. Nobody has reviewed any of these packs.
