# hcp-manager

The independent application that manages documents, sources, knowledge packs and the search index for hardware constraint packs
([HCP.md](HCP.md)). The Skill, the checkers and fastroute call it through its command line (or use the library, which is the same
code); none of them keeps a cache, a download routine or an index of its own.

It does not judge what a document means, it does not produce rules, and nothing that compiles, validates or exports a pack reads it.

## What is a fact and what is derived

| | What | Where | If it is lost |
|---|---|---|---|
| **fact** | the inventory of documents (version, level, devices, pinned SHA-256) | `<workspace>/sources.yaml` | it is the record: keep it in version control |
| **fact** | the knowledge packs (HCP YAML) | `<workspace>/packs/<id>/<version>/pack.yaml` | it is the record |
| **fact** | the configuration of the embedding provider | `<workspace>/embedding.yaml` (optional) | the local default is used |
| derived | the cached documents, named by digest | `<workspace>/cache/<DOC>/<sha256>.<ext>` | `sources sync` fetches them again and checks the digest |
| derived | the index: text blocks and vectors, in the store backend in force: SQLite (default) or JSON Lines | `<workspace>/derived/index.db` or `<workspace>/derived/jsonl/` | `index build` makes it again from the cache |

A workspace is one directory. Documents, packs and indexes of different workspaces never meet; inside a workspace packs are kept by id and
version, blocks by document and the digest of the file they were cut from, and vectors by the identity of the embedding configuration.

Core operations (reading, listing, showing, exporting and importing packs, `evidence verify` of what is cached, everything in
`fastroute constraints`) need **no network, no embedding model and no index database**. Deleting `derived/` loses nothing that is not
rebuilt by one command.

## Commands

Exit codes as in fastroute: 0 ok, 1 failed, 2 usage, 3 input, 4 output refused, 5 done but something could not be done. `--workspace DIR`
(default `$HCP_MANAGER_HOME`, else `./.hcp`) and `--json` work everywhere.

| Command | |
|---|---|
| `init`, `info` | create the directories; the contract (`interface`, commands, capabilities, exit codes) |
| `sources add\|import\|list\|resolve\|sync\|check` | the inventory; `resolve --device PART` lists the documents for the exact device, best source first, and names similar models that were excluded; `sync` downloads and pins digests (a changed file is reported, not adopted, without `--accept`) |
| `index build [DOC...] [--force] [--no-vector] [--authorize-remote]` | makes what is missing or stale, keeps what is current |
| `index rebuild`, `index status` | delete the derived index of the backend in force and build it again; what is built, what is stale, and why; backend, profile and the generations of vectors |
| `index prune` | remove the vectors of every embedding configuration except the one in force (the only way an older generation goes away) |
| `--profile NAME`, `--backend sqlite\|jsonl` (any command) | the embedding profile and the store backend for this command (else `active_profile` / `store.backend` of `embedding.yaml`, else the local default and SQLite) |
| `search WORDS --device --revision --version --source-level --limit --mode keyword\|vector\|hybrid [--authorize-remote]` | filtered by device, version and source level **before** it is ranked; ordered by source priority (official manual, guide, devkit, open board, community), then by score |
| `chunk show CHUNK_ID` | the block of a hit: document, version, level, devices, page, section, the text and its digest, the cached file and its digest, whether the index of it is current |
| `embedding show\|test` | the configuration in force and whether text would leave this machine; a test embedding |
| `evidence verify PACK` | every source, digest, version, block id and quotation of a pack against the cached documents (current blocks only) |
| `packs list\|show\|create\|import\|export\|resolve` | see below |
| `packs embed --board ID --request FILE --out DIR` | a development board pack embedded in a carrier as a fixed module: the carrier pack, its IR, the differences and a report. [hcp-board-embedding.md](hcp-board-embedding.md) |
| `packs builtin list\|show\|verify\|install` | the packs that ship with the software (a release directory): verify and install them offline, with the exact dependencies; a locally changed copy is never overwritten silently. [hcp-pack-release.md](hcp-pack-release.md) |
| `release seal\|index\|build` | for pack authors and release builders: write the digests of a pack's manifest, the committed catalogue (`index.yaml`) and the deterministic release directory. [hcp-pack-release.md](hcp-pack-release.md) |

### Packs

* `packs create --id vendor/device --device PART --vendor V [--pack-version 0.1.0] [--revision R] [--title T] [--language en]` writes a skeleton to
  `packs/<id>/<version>/pack.yaml`: the metadata, the device and the documents of the inventory that are for this exact device and have a digest. It has
  no pins and no rules and says it is not valid; it is never overwritten (another version is another file).
* `packs list` shows every version of every pack (kind, and what it inherits from); `show` and `evidence verify` take a path, an id (the highest version) or `id@version`.
* `packs resolve PACK [-o FILE]` merges a pack that inherits (`pack.upstream`, HCP 1.1: module from chip, board from module or chip) with the exact upstream packs of the workspace into one flat pack. `evidence verify`, `packs list` and `packs show` resolve first, so inherited evidence is checked against the documents too.
* `packs export PACK --to DIR` writes the pack, `sources-used.yaml` (only the documents it cites) and `export-manifest.json` with digests.
* `packs import DIR_OR_FILE` takes such a directory (digests must match: an export changed after it was written is refused) or a pack file. The pack
  must be valid. The documents it cites are added to the inventory; a document that is already there with another version, level, devices or digest is a
  conflict, and nothing is imported (the inventory is not changed by an import). An existing pack of the same id and version is replaced only with `--force`.

## The index

The derived index holds `documents` (per document: the digest of the source, the chunking strategy, a signature of its inventory entry, the number of blocks), `chunks` (the blocks, see below), `embedders`, `doc_vectors` and `vectors`. No timestamps: building twice gives the same content. It lives in one of two **store backends** behind one interface (`store::Backend`); each has its own location and is built, checked and removed on its own:

| Backend | Where | What it is |
|---|---|---|
| `sqlite` (default) | `derived/index.db` | one SQLite file |
| `jsonl` | `derived/jsonl/` | plain JSON Lines files (`documents`, `chunks`, `embedders`, `doc_vectors`, `vectors`) and `store.json`, a manifest with the format version and the SHA-256 and line count of every file. It is the reference second format: it shows that the stored format is replaceable and can be read with any tool. It is read into memory when opened and rewritten on change: for small workspaces and debugging, **not** a fast store and not an ANN index |

The backend is chosen by `--backend`, else `store.backend` of `embedding.yaml`, else `sqlite`. The same queries give the same candidates, scores and references on both (compared exactly in `tests/backends.rs`, with a fixed fake embedding); a hit goes back to document, version, page and section with `chunk show` on either. A backend is never converted into the other: build it from the cache. **A store of another layout version, or a damaged one** (a file that does not match its checksum, a database that fails its check, fewer vectors than the record says) is reported by `index status` with the reason and replaced by `index build`, never migrated in place: it holds nothing the cache cannot rebuild.

**A block** (`schemas/chunk-v1.schema.json`, printed by `chunk show`): `chunk_id` (`CHK-<DOC>@<first 8 digits of the document's SHA-256>:p<page>.<n>`),
`document_id`, `document_version`, `level` (source level), `applies_to` (the devices), `page`, `ordinal`, `section`, `text`, `text_sha256`. Blocks are cut at
paragraph boundaries (about 400 to 1500 characters; strategy `paragraph-400-1500-v1`) from the text of the page; a changed document gives new ids.

**What makes it stale** (`index status` names the reason; `index build` makes again exactly that):

| Change | Effect |
|---|---|
| the cached document changed (another digest) | blocks and vectors of that document are stale |
| the chunking strategy of the program changed | blocks stale |
| the inventory entry changed (version, level, devices) | blocks stale: they carry those fields |
| the embedding configuration changed (provider, model, version, dimension, command, URL) | vectors stale; blocks kept; the vectors of the old configuration are **kept** as an older generation (until `index prune`), never mixed in |
| the store holds fewer vectors than it recorded | the vectors of that document are stale (`damaged`); blocks kept |
| the store is damaged or of another layout version | it is replaced by the next build (said in the status and in the build report) |

Search and `evidence verify` use only the blocks of a current index. Text of an older version of a document is never taken for the text of the cached one.

## Vectors and hybrid search

**What the vectors are for.** To find candidate passages that the words of a query do not find. A vector, a similarity or a rank is never evidence and never
part of a pack, an IR or a verdict (`hcp_version` 1.0 packs with such fields are refused: `vector_field`; `fr-constraint` and `fastroute` do not link this
application or the database, which a test checks). A hit carries `hit_semantics`, `matched_by` (`keyword` and/or `vector`), `keyword_score` and `vector_similarity`.

**Providers** (`<workspace>/embedding.yaml`, schema `schemas/embedding-config-v1.schema.json`, an annotated example in [`apps/hcp-manager/config.example.yaml`](../apps/hcp-manager/config.example.yaml); without the file the first is used):

| Provider | What | Leaves the machine? |
|---|---|---|
| `hashing` (default) | built in, deterministic, local: words, word pairs and character trigrams hashed into `dim` (default 384) signed buckets. **It is lexical**: it helps with spelling variants and parts of words and identifiers, not with synonyms. No model is bundled. | no |
| `command` | a program you name (argument list, no shell, run in the workspace directory): `{"texts": [...]}` on stdin, `{"model", "version", "dim", "vectors"}` on stdout; model, version and dimension must be what the configuration says | what the program does is its own business |
| `openai` | an OpenAI-compatible embeddings service (below) | **only if the host is not this machine** |
| `http` | an OpenAI-style embeddings endpoint (`{"model", "input"}`; `{"data": [{"index", "embedding"}]}` or `{"embeddings": [...]}`), via `curl`; the API key comes from the environment variable named by `api_key_env`, never from a file. Kept as it was; `openai` is the one with retries, batches and a checked dimension | **only if the host is not this machine** |

**Remote embedding is refused unless all of these hold**: `allow_remote: true` in `embedding.yaml`; `--authorize-remote` on the command that would send; and
`embed_remote: true` on the inventory entry of **each document** that may be sent. The query of a search is sent only under the first two. An endpoint on
localhost is local. With any of them missing nothing is sent: `index build` says so (exit 5) and the keyword index is complete; a hybrid search says it fell
back to keywords. `embedding show` states whether text leaves the machine.

**Modes.** `keyword` (default; unchanged by the vector layer). `vector`: an error when there are no current vectors. `hybrid`: a block's score is its keyword
score relative to the best keyword hit plus `vector_weight` times its similarity above a noise floor (0.15) (default weight 0.5 for `hashing`, 1.0 for a
plugged-in model; configurable); hits are ordered by source level first, then by score; documents without current vectors are searched by keyword only (a
warning); with no usable vectors at all it is a keyword search and says so (`degraded`). Filters (device, revision, version, source level, status, current
index) are applied **before** any ranking, to the blocks that are candidates for both rankings, so a similar model's document is not returned however alike
its words are. The vector store sits behind the `VectorIndex` trait (`src/store.rs`); the flat scan over the filtered candidates is what ships.

**Measured** (ESP32-S3-WROOM-1-N8R8 workspace, 795 blocks, default configuration; `hardware-packs/espressif/esp32-s3-wroom-1-n8r8/eval/`):

| | the 84 quotations of the pack, first hit / first three | 12 questions in other words, first three |
|---|---|---|
| keyword | 67 / 84 | 9 |
| vector (hashing) | 46 / 62 | 8 |
| hybrid | 66 / 84 | 8 |

On these documents the default vectors do not add to the keywords (the questions are a small sample: one question is noise); they find spelling variants that
the words miss (shown on a fixture), and the layer is where a real model is plugged in. No semantic model was bundled, downloaded or tested here.

### The OpenAI-compatible provider, profiles and generations

* **Request**: `POST <endpoint>` with `{"model", "input": [texts], "encoding_format": "float"}` (and `dimensions` with `send_dimensions: true`), `Content-Type: application/json` and, when a key is configured, `Authorization: Bearer <key>`. **Answer**: `{"data": [{"index", "embedding": [numbers]}]}`; items may come in any order, `index` says which text each belongs to, and the `index` values must be `0..n-1` once each. An empty or malformed answer, a missing `data`, duplicate or out-of-range indices, or a vector of another size than `dim` is an error that says why, and nothing is stored.
* **Address**: `base_url` with or without the version prefix (`https://h`, `https://h/v1`, `https://h/v1/embeddings` all reach `https://h/v1/embeddings`; a prefix that already ends in a version such as `/api/paas/v4` is kept and `/embeddings` appended); `embeddings_path` replaces everything after the host for services with another path. No credentials or query in the address. `embedding show` prints the endpoint that will be used.
* **`dim` is required and checked on every answer.** A service that returns another size (a different model, or `dimensions`) is an error that tells you to set `dim` and build again; vectors of another size are never mixed in.
* **Batches, time, retries**: `batch` texts per request (default 32, at most 2048), `timeout_s` (default 60), `retries` (default 2) after a timeout, a **429** or a **5xx** answer, waiting `retry_backoff_ms` (default 500, doubled each time; `Retry-After` honoured up to 30 s). Nothing else is retried: 401/403 (the credentials were refused; names where the key comes from), 404 (says to check `base_url` and `embeddings_path`), other 4xx (the service's message, cut to 160 characters, with the key blanked out), a refused connection (`cannot be reached`).
* **The key** comes from the environment variable named by `api_key_env` or from the output of `api_key_command` (a program and arguments, no shell); never from the configuration, never stored. It travels to curl in a file in a private directory, not on a command line, and appears in no output, error message or file (tested by searching every output and every file of the workspace for it). `embedding show` names the source and prints only the program name of a command, not its arguments.
* **Remote**: unchanged. `allow_remote: true` **and** `--authorize-remote` **and** `embed_remote: true` on each document; with any missing nothing is sent (tested: zero requests reach the mock). A service on localhost is local.
* **Profiles**: `embedding.yaml` may define several named `profiles` with one `active_profile`; `--profile NAME` picks another for one command. A file with one provider written flat is the profile `default`. An unknown name, several profiles with none named, or a flat provider together with `profiles` is an error, never a fallback to another profile.
* **Generations**: the vectors belong to the configuration (its key `provider:model:version:dim:digest`, which includes the endpoint) they were made with. Switching the profile, the model, the dimension or the address makes a **new generation**: the old one is **kept** (searches use only the one in force), and `index status` lists every generation. Nothing deletes an older generation but `index prune`. There is no online migration: a new generation is made from the cache.
* **Offline**: with the service unreachable, `index build` makes the blocks (keyword index complete) and exits 5 for the vectors; `search --mode hybrid` is a keyword search and says why; `--mode vector` is an error. Compile, validate and everything in `fastroute constraints` read neither the service nor the index.

**What has been verified** (see `tests/openai.rs`, `tests/backends.rs`): this program's requests and its handling of answers, errors, limits and timeouts, against a **mock of the protocol** (a real HTTP service on this machine that records every request), for both store backends. **No real OpenAI or other third-party service was called**, so how any of them behaves (their limits, their exact error bodies, their vector sizes and quality) is not verified here; a real service is a configuration you test with `embedding test`. The vectors in the tests are fixed fakes: retrieval quality with a real model is not measured.

## The contract

`hcp-manager info --json`: `"interface": "1"`. Within interface 1 changes are **additive**: new commands, new options, new JSON fields (and `format_version` of a
result goes up in its minor part); nothing is removed, renamed or given another meaning. A change that breaks that is `interface: "2"`, and a program that
speaks 1 refuses it (the Skill's `doctor` does). `capabilities` says what exists (`keyword_search`, `vector_index`, `hybrid_search`, `embedded_store`, `vector_backends`, `embedding_profiles`,
`embedding_providers`, `semantic_model_bundled`, `remote_embedding_by_default`, `builtin_packs`, `pack_inheritance`, `pack_manifest_version`, `release_format`, `pdf_text`, `network_fetch`); `pdf_text` needs `pdftotext` and `network_fetch`
needs `curl`. A command line is the interface: there is no server and no GUI. The library (`hcp_manager::{inventory, sources, index, search, evidence, packs, embed, store}`)
is the same code and has the same behaviour; it is not a separate stable API.
