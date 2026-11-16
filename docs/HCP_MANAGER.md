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
| derived | the index: text blocks and vectors, in one SQLite file | `<workspace>/derived/index.db` | `index build` makes it again from the cache |

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
| `index rebuild`, `index status` | delete everything derived and build it again; what is built, what is stale, and why |
| `search WORDS --device --revision --version --source-level --limit --mode keyword\|vector\|hybrid [--authorize-remote]` | filtered by device, version and source level **before** it is ranked; ordered by source priority (official manual, guide, devkit, open board, community), then by score |
| `chunk show CHUNK_ID` | the block of a hit: document, version, level, devices, page, section, the text and its digest, the cached file and its digest, whether the index of it is current |
| `embedding show\|test` | the configuration in force and whether text would leave this machine; a test embedding |
| `evidence verify PACK` | every source, digest, version, block id and quotation of a pack against the cached documents (current blocks only) |
| `packs list\|show\|create\|import\|export` | see below |

### Packs

* `packs create --id vendor/device --device PART --vendor V [--pack-version 0.1.0] [--revision R] [--title T] [--language en]` writes a skeleton to
  `packs/<id>/<version>/pack.yaml`: the metadata, the device and the documents of the inventory that are for this exact device and have a digest. It has
  no pins and no rules and says it is not valid; it is never overwritten (another version is another file).
* `packs list` shows every version of every pack; `show` and `evidence verify` take a path, an id (the highest version) or `id@version`.
* `packs export PACK --to DIR` writes the pack, `sources-used.yaml` (only the documents it cites) and `export-manifest.json` with digests.
* `packs import DIR_OR_FILE` takes such a directory (digests must match: an export changed after it was written is refused) or a pack file. The pack
  must be valid. The documents it cites are added to the inventory; a document that is already there with another version, level, devices or digest is a
  conflict, and nothing is imported (the inventory is not changed by an import). An existing pack of the same id and version is replaced only with `--force`.

## The index

One SQLite file with these tables: `documents` (per document: the digest of the source, the chunking strategy, a signature of its inventory entry, the number
of blocks), `chunks` (see the block schema below), `embedders`, `doc_vectors` and `vectors`. No timestamps: building twice gives the same file content.

**A block** (`schemas/chunk-v1.schema.json`, printed by `chunk show`): `chunk_id` (`CHK-<DOC>@<first 8 digits of the document's SHA-256>:p<page>.<n>`),
`document_id`, `document_version`, `level` (source level), `applies_to` (the devices), `page`, `ordinal`, `section`, `text`, `text_sha256`. Blocks are cut at
paragraph boundaries (about 400 to 1500 characters; strategy `paragraph-400-1500-v1`) from the text of the page; a changed document gives new ids.

**What makes it stale** (`index status` names the reason; `index build` makes again exactly that):

| Change | Effect |
|---|---|
| the cached document changed (another digest) | blocks and vectors of that document are stale |
| the chunking strategy of the program changed | blocks stale |
| the inventory entry changed (version, level, devices) | blocks stale: they carry those fields |
| the embedding configuration changed (provider, model, version, dimension, command, URL) | vectors stale; blocks kept |
| the store file is damaged or of another layout version | it is replaced (said in the build report) |

Search and `evidence verify` use only the blocks of a current index. Text of an older version of a document is never taken for the text of the cached one.

## Vectors and hybrid search

**What the vectors are for.** To find candidate passages that the words of a query do not find. A vector, a similarity or a rank is never evidence and never
part of a pack, an IR or a verdict (`hcp_version` 1.0 packs with such fields are refused: `vector_field`; `fr-constraint` and `fastroute` do not link this
application or the database, which a test checks). A hit carries `hit_semantics`, `matched_by` (`keyword` and/or `vector`), `keyword_score` and `vector_similarity`.

**Providers** (`<workspace>/embedding.yaml`, schema `schemas/embedding-config-v1.schema.json`; without the file the first is used):

| Provider | What | Leaves the machine? |
|---|---|---|
| `hashing` (default) | built in, deterministic, local: words, word pairs and character trigrams hashed into `dim` (default 384) signed buckets. **It is lexical**: it helps with spelling variants and parts of words and identifiers, not with synonyms. No model is bundled. | no |
| `command` | a program you name (argument list, no shell, run in the workspace directory): `{"texts": [...]}` on stdin, `{"model", "version", "dim", "vectors"}` on stdout; model, version and dimension must be what the configuration says | what the program does is its own business |
| `http` | an OpenAI-style embeddings endpoint (`{"model", "input"}`; `{"data": [{"index", "embedding"}]}` or `{"embeddings": [...]}`), via `curl`; the API key comes from the environment variable named by `api_key_env`, never from a file | **only if the host is not this machine** |

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

## The contract

`hcp-manager info --json`: `"interface": "1"`. Within interface 1 changes are **additive**: new commands, new options, new JSON fields (and `format_version` of a
result goes up in its minor part); nothing is removed, renamed or given another meaning. A change that breaks that is `interface: "2"`, and a program that
speaks 1 refuses it (the Skill's `doctor` does). `capabilities` says what exists (`keyword_search`, `vector_index`, `hybrid_search`, `embedded_store`,
`embedding_providers`, `semantic_model_bundled`, `remote_embedding_by_default`, `pdf_text`, `network_fetch`); `pdf_text` needs `pdftotext` and `network_fetch`
needs `curl`. A command line is the interface: there is no server and no GUI. The library (`hcp_manager::{inventory, sources, index, search, evidence, packs, embed, store}`)
is the same code and has the same behaviour; it is not a separate stable API.
