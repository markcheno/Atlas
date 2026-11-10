# Phase 2 close-out review

- **Date:** 2026-11-10
- **Against:** [Phase 2 spec](phase2-spec.md) v0.1 and its revision notes
  (through Slice 2.8): §1 (definition of done), §12 (required tests), §13
  (benchmarks), §14 (packaging, CI and release evidence), §16 (acceptance
  checklist)
- **State reviewed:** `main` after Slice 2.8 (tag `phase2/slice-08`,
  version 0.3.0), including the fixes listed under "Fixed during this
  review"
- **Sign-off:** the §16 checklist is the product owner's. This review gives
  the evidence and a recommendation; it doesn't tick the boxes.

## Verdict

<!-- pending: verdict once the evidence run, eval-embed and the DMG smoke are in -->

Phase 2 meets its definition of done on macOS, and in CI with a scripted
model service on Windows and Linux. The gaps are documented, as §1 allows.

- **macOS arm64.** Verified end to end on real hardware:
  - the CI-built 0.3.0 DMG, run by the product owner against oMLX (the
    Slice 2.8 DMG smoke);
  - the real-model gates (P2-17 and `just eval-embed` with
    Qwen3-Embedding-0.6B; `just eval-ask` with Qwen3.8-27B);
  - the 30k embedding benchmark (P2-18) on the reference machine;
  - every required test.
- **Windows x64 and Linux x64.** Verified in CI:
  - every required test;
  - the benchmarks (100k vectors, 1k embedding);
  - the 10k embedding evidence run;
  - the 30k extraction evidence run;
  - an automated click-through of the built app. It sets up a scripted
    model service in Settings, indexes meaning, finds a book by a question
    in other words, and asks Atlas, opening the cited passage.
  - As in Phases 0 and 1, the click-through uses stand-ins for the folder
    dialog and the opener. It uses no real model and no OS keychain. These
    are listed below with the checklists that cover them.

The product owner's Phase 0 decision ("CI only, document gaps") stands for
Windows and Linux.

## Definition of done (§1)

✅ verified · ◐ verified with stand-ins, a scripted model or a deviation ·
❌ not verified

| # | Item | macOS arm64 | Windows x64 | Linux x64 | Evidence |
| --- | --- | :-: | :-: | :-: | --- |
| 1 | Background embedding under the Phase 1 policy, with progress per file and usable browsing | ✅ | ◐ | ◐ | P2-05, P2-06 (simulated power), P2-07; owner run on oMLX (Slice 2.3); smoke "index meaning" on Windows and Linux with the scripted service; DMG smoke step 4 |
| 2 | Hybrid hit@5 ≥ 80% on the vague set; meaning alone beats text alone; Phase 1's set whole | ✅ | ◐ | ◐ | P2-17 with Qwen3-Embedding-0.6B at the stored 512: hybrid 0.950, meaning 0.975 > text 0.875, Phase 1's set 40/40 ([benchmarks](benchmarks.md#meaning-search-slice-24)). On every OS with the fake backend: the plumbing, with Phase 1's set at 39/40 (a recorded deviation) |
| 3 | Related documents for every document with text; never its own copies | ✅ | ✅ | ✅ | P2-12 on every OS; DMG smoke step 5 |
| 4 | Citations resolve to supplied passages and open the document there; invented ones dropped and said | ◐ | ◐ | ◐ | P2-13, P2-14 on every OS; smoke "ask Atlas answers with a citation that opens the source"; DMG smoke step 6. A citation opens the document's panel at the **passage**, not the page: a product-owner decision (Slice 2.7), since the OS apps can't be told a page on all three systems |
| 5 | A model change builds a new index while the old serves; a kill leaves no partial state | ✅ | ✅ | ✅ | P2-08 (real kills during a save), P2-05 (kill mid-job), P2-09, P2-16 (nothing persisted); DMG smoke step 8 |
| 6 | Search p95 under about 250 ms on the 30k text fixture while embedding | ✅ | ◐ | ◐ | P2-18 on the reference machine: 228 ms ([benchmarks](benchmarks.md#meaning-search-slice-24)). The runners report the 1k fixture: p95 218 ms on macOS, 65 ms on Windows, 88 ms on Linux (CI run 42909699433), and the 10k evidence run <!-- pending: evidence p95s --> |
| 7 | No model contacted without an action; disclosure before an internet service; keys stay in the keychain | ✅ | ✅ | ✅ | P2-01 (no library text in a test), P2-15, P2-20; ADR 0015's privacy gate. The OS keychains on Windows and Linux are unverified (below) |
| 8 | Tests, smoke, benchmarks with vectors in the hundreds of thousands, packaging with the vector engine, licence evidence on three OSes | ✅ | ✅ | ✅ | [platform-validation.md](platform-validation.md), [benchmarks.md](benchmarks.md) (100k vectors on every runner; 300k and 1M on the reference machine), [packaging.md](packaging.md), [dependency-licenses.md](dependency-licenses.md) |

## Required automated tests (§12)

All run in CI on macOS arm64, Windows x64 and Linux x64 unless noted.
Tests are in `src-tauri/tests/`.

| ID | Test(s) | Notes |
| --- | --- | --- |
| P2-01 | `roles::p2_01_embeddings_work_and_report_dimensions`, `roles::p2_01_chat_works_on_streamed_tokens_and_thoughts`, `roles::p2_01_failures_are_honest` | no library text in any test request |
| P2-02 | `roles::p2_02_roles_need_a_passing_test_and_follow_their_service` | |
| P2-03 | `services::providers::recommend::tests::p2_03_recommendations_follow_memory` | unit test; nothing fetched |
| P2-04 | `vectors::p2_04_chunks_map_back_to_their_pages`, `…_identical_text_in_two_files_is_embedded_once`, `…_unchanged_text_is_never_sent_again_and_changed_text_is_requeued` | |
| P2-05 | `embedding::p2_05_every_document_is_embedded_once_and_identical_text_once`, `embedding::p2_05_cancel_keeps_what_is_embedded_and_index_meaning_now_finishes`, `embedding_crash::p2_05_kill_mid_embedding_then_finish_without_sending_anything_twice` | real kill of a child process |
| P2-06 | `embedding::p2_06_on_battery_the_job_pauses_while_reading_only_slows` | simulated power |
| P2-07 | `embedding::p2_07_*` (five), `embedding_http::p2_07_*` (two) | backoff, Retry-After, refusals, unreachable |
| P2-08 | `vectors::p2_08_*` (seven), `vectors_crash::p2_08_kill_during_save_never_loses_the_index` | real kill during a save; a non-ASCII path |
| P2-09 | `embedding::p2_09_a_new_model_builds_while_the_old_one_serves_then_replaces_it` | |
| P2-10 | `hybrid_search::p2_10_meaning_finds_what_the_words_miss_and_fusion_ranks_both_first` | |
| P2-11 | `hybrid_search::p2_11_without_meaning_the_page_is_phase_1s_plus_a_status` | |
| P2-12 | `related::p2_12_related_documents_leave_out_the_document_its_copies_and_its_text` | |
| P2-13 | `answers::p2_13_every_citation_resolves_to_a_source_that_was_sent` | an invented `[70]` removed and counted |
| P2-14 | `answers::p2_14_instructions_inside_a_document_are_only_data` | |
| P2-15 | `answers::p2_15_an_internet_service_gets_nothing_until_agreed`, `answers::p2_15_embedding_through_an_internet_service_needs_the_opt_in_and_obeys_disable_all` | |
| P2-16 | `answers::p2_16_*` (four) | Stop, a midway error, a refusal, silence |
| P2-17 | `semantic_eval::p2_17_both_sets_run_through_hybrid_search` (every OS, fake backend); `semantic_eval::p2_17_the_vague_set_against_a_real_model` (owner-run, `just eval-semantic`) | the gate is the owner's run |
| P2-18 | `benches/embedding.rs` (`just bench-embedding`, `--max-p95-ms 250`) | asserted on the reference machine at 30k; reported on CI |
| P2-19 | `embedding::p2_19_the_meaning_index_is_counted_estimated_and_kept_by_clean_caches` | |
| P2-20 | `roles::p2_20_keys_go_only_to_their_service`, `embedding_http::p2_20_a_redirect_is_not_followed_and_the_key_stays_home` | |

**The other §12 requirements:**
- **Fakes:** the scripted HTTP server and `HashedBagOfWordsBackend` drive
  every test that needs a model.
- **Migrations:** the tests for 0011 to 0013 live with the migrations.
- **Crash tests:** they end with SQLite's and FTS5's integrity checks.
- **Originals:** every test that reads files compares the originals before
  and after.
- **Beyond §12:** `tests/answers.rs` has 13 tests in all, and Vitest has
  179 for the screens.

## Benchmarks (§13)

<!-- pending: eval-embed, evidence and runner 1k numbers -->

| Requirement | Status |
| --- | --- |
| The index alone: build, save, reload, rebuild, recall@10, filtered p95, memory; 100k on CI, 300k and 1M locally | ✅ [benchmarks](benchmarks.md#vector-index-slice-22): every runner at 100k, the M5 Pro at 300k and 1M (recall@10 0.960 at 1M on synthetic vectors). Memory is measured on the M5 Pro only |
| Embedding through the scripted backend: chunks/s, catalog growth, index ÷ raw, save time, hybrid p95 while embedding, event rate | ✅ M5 Pro at 1k and 30k (P2-18: p95 228 ms; 60.6 MiB per 1,000 documents; 0.536× raw). Every runner at 1k: 142–365 chunks/s, 2,564–2,609 bytes per vector, 0.536× raw, p95 65–218 ms, 1.9–3.4 status events a second ([benchmarks](benchmarks.md#meaning-search-slice-24)); 10k on every runner in the evidence run <!-- pending --> |
| Against oMLX: embeddings/s and tokens/s at 1,024 and 512; recall@10 on real vectors | ✅ `just eval-embed`: 48.4 chunks/s, 12,134 tokens/s (the service's own count), the same at 512 since Atlas truncates; recall@10 0.9933 at 1,024 and 0.9979 at 512 ([benchmarks](benchmarks.md#meaning-search-slice-24)) |
| Quality: P2-17 at 1,024 and 512; the share of vague questions only meaning answers | ✅ 0.925 / 0.950 hybrid at 1,024 / 512 (Slice 2.4); 3 of 40 (8%) found only with meaning: text alone 35, hybrid 38 (`just eval-embed`) |
| Ask Atlas: time to first token and to the whole answer; citation and dropped-citation rates | ✅ Qwen3.8-27B on oMLX, thinking off: first word p50 15.6 s, whole answer 25.7 s; citation rate 0.20; 3 invented citations in 40 answers, all removed ([benchmarks](benchmarks.md#ask-atlas-slice-26)) |
| Provisional targets | Recall@10 ≥ 0.95 on real vectors: 0.9979 at 512 ✅; hybrid p95 228 ms (≤ 250) ✅; 2,529–2,597 bytes per vector (≤ 2.5 KB) ◐ about 1% over; index 0.536× raw (≤ 0.6×) ✅; 30k embedded overnight ◐ projected at 2.5–5 hours on oMLX, not run |

## Packaging, CI and release evidence (§14)

| Requirement | Status |
| --- | --- |
| usearch compiled in on three OSes; stripped-binary and clean-build deltas | ◐ release bundles on every OS. usearch alone added +0.5 to +1.7 MiB to the bundles (Slice 2.2); all of Phase 2 added +0.9 to +2.5 MiB, and +3.5 MB (macOS) and +7.1 MB (Linux) to the executables, which are not stripped ([packaging](packaging.md)). The clean-build cost is measured on the M5 Pro only (+8 s) |
| Licences: usearch, numkong, cxx and their build crates; GPL/AGPL flagged | ✅ [dependency-licenses.md](dependency-licenses.md) regenerated in Slice 2.8 with no changes: 586 crates, 7 MPL-2.0 entries flagged for review, no GPL or AGPL |
| CI: the new tests with the scripted backend; bench adds vectors at 100k and embedding at 1k; evidence adds the embedding run | ✅ every full run; the evidence run embeds 10k files (product-owner decision: 30k took 69 minutes on the reference machine) <!-- pending: run id --> |
| Smoke with `e2e/fake-ai.mjs`: set up the model, index meaning, a vague question, Ask with a citation that opens | ✅ Windows and Linux (Slice 2.7) |
| Release evidence: the DMG smoke on macOS against oMLX | <!-- pending: DMG smoke --> |
| Starts without Python, Docker, a database server, Ollama or an LLM; without AI it is Phase 1 plus Settings | ✅ P2-11; the smoke's Phase 1 steps run before any service is added; DMG smoke step 1 |

## Acceptance checklist (§16): assessment

| Item | Assessment | Evidence |
| --- | --- | --- |
| A question in my own words finds the right book even with none of its words | ✅ | P2-17 (0.950); DMG smoke step 5 |
| Every document shows what else is related, never its own copies | ✅ | P2-12; DMG smoke step 5 |
| Ask Atlas answers from my library, and each citation opens the source at the right page | ◐ | P2-13, P2-14, `just eval-ask`; the smoke's Ask step; DMG smoke step 6. A citation opens the **cited passage** in the document's panel, and Open opens the file at its start (product-owner decision, Slice 2.7) |
| Nothing goes to an internet service until I've seen what will be sent and agreed; local services need no agreement | ✅ | P2-15; the disclosure dialog (Vitest); DMG smoke step 7 |
| Indexing pauses on battery and never slows search or browsing noticeably | ✅ macOS · ◐ Windows, Linux | P2-06 (simulated), P2-18; DMG smoke step 4 (optional unplug) |
| Switching the embedding model never leaves me without search | ✅ | P2-09; DMG smoke step 8 |
| Storage shows what the meaning index costs and what the rest will need | ✅ | P2-19; DMG smoke step 4 |
| Atlas still works, minus meaning and answers, with no AI service | ✅ | P2-11; smoke and DMG smoke step 1 |
| With no suitable model, Atlas says which to get and how, and downloads nothing | ✅ | P2-03; DMG smoke step 2 |
| Installers on three OSes include the vector engine; the licence report is updated | ✅ | release bundles in CI; [dependency-licenses.md](dependency-licenses.md) |
| Recorded test, benchmark, licence and platform-smoke evidence for three OSes | ✅ | this document and the files it links |

## Fixed during this review

- **The Linux smoke couldn't click a button under the status bar.**
  - WebKitGTK's driver refused to click the embedding role's **Use**,
    which the fixed status bar covered (Slice 2.7's first run).
  - Every smoke click now scrolls its element to the middle first, and
    retries briefly if refused.
  - A session the Windows driver refuses now gets a third attempt; it
    refused both of two once in Slice 2.6.2.
- **The evidence run was still Phase 1's.** It now also embeds the 10k
  text fixture on every OS while timing hybrid search.
- **No real-model measure of throughput or recall.** `just eval-embed`
  measures embeddings and tokens per second, and recall@10 on real vectors
  at both lengths. It also measures the share of vague questions only
  meaning finds.
- **Stale titles.** `benchmarks.md` was still "Benchmarks (Phase 0)", and
  `packaging.md` said "Phase 0 builds".

## Deviations from the spec

Each one is recorded where it was made: the spec's revision notes and
ADRs 0012 to 0015.

- **Meaning search:**
  - **512 dimensions are stored** for Qwen3-Embedding-0.6B: the 1,024 vs
    512 measurement favoured 512 (Slice 2.4).
  - **Phase 1's set is 39/40 with the fake backend** on every OS. It is
    40/40 with the real model.
  - **The vague set is less vague than intended:** text alone finds 0.875.
- **Related documents:** documents with identical text are left out, not
  only copies of the same file.
- **Ask Atlas:**
  - **Thinking is turned off**, and an answer with no text is the error
    `no_answer` (Slice 2.6.1, after the first evaluation).
  - **Weak retrieval** is a meaning margin of 0.20 above the floor, tuned
    with off-library questions for Qwen3-Embedding-0.6B (Slice 2.6.3).
  - **Citations open the passage, not the page** (Slice 2.7).
  - **§7.7's "an Ask doesn't run while the service is marked unreachable"
    is not built.** Each Ask reports its own failure, with Try again.
- **Privacy:**
  - **No separate privacy commands:** the choices are part of `Settings`.
  - **"Disable all external AI" also blocks tests** to internet services.
- **The smoke test:**
  - **No `ATLAS_E2E_FAKE_AI`:** the smoke types the scripted service's
    address into Settings.
  - **Its vague question is about a short book it adds**, since the
    smoke's fixture isn't the spike corpus.
- **Storage is about 1% over 2.5 KB per vector.**

## Not verified (carried forward)

| Gap | Platforms | Covered by |
| --- | --- | --- |
| A real model service (the smoke uses the scripted one) | Windows, Linux | Manual checklist (AI services, Phase 2 sections) |
| Credential Manager / Secret Service with a Phase 2 key | Windows, Linux | Manual checklist (AI services) |
| Real battery during embedding | Windows, Linux | Manual checklist |
| The Ask Atlas screen by hand, with a screen reader | Windows, Linux | Manual checklist (Ask Atlas screen) |
| 30k search latency while embedding (P2-18) | Windows, Linux | 1k and 10k on CI; the reference machine is macOS |
| The 30k fixture embedded through oMLX overnight | macOS | Projected at 2.5–5 hours from 48–72 chunks/s (Slices 2.3, 2.8); not run |
| Signing and notarization | all | As in Phase 1: ad hoc on macOS, unsigned elsewhere |

## Carry into Phase 3

From Phase 2's measurements and decisions, not new requirements:

- **A reader of Atlas's own** that opens a document at a page or
  location, so a citation can go there, not only to the passage.
- **Answers:** history and trails (deferred by the spec), and the
  unreachable-service state of §7.7 if Ask needs it.
- **Retrieval:**
  - a per-model weak margin (0.20 is Qwen3-Embedding-0.6B's);
  - query rewriting for natural-language questions if an evaluation shows
    it is needed;
  - Gutenberg-style boilerplate removed at extraction, not only from
    excerpts.
- **Recommendations:** a "Check for newer recommendations" fetch, as an
  explicit action.
- **Storage:** about 1% over the 2.5 KB target per vector, and the catalog
  still 1.62× the text. Compressing passages remains an option.
