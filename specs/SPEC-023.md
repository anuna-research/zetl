---
title: "SPEC-023: zetl bench — Search Quality Benchmarking Harness"
version: 0.1.0
status: draft
date: 2026-04-07
audience: agent, human
parent: SPEC-002
related:
  - SPEC-013
  - SPEC-018
dependencies:
  - tantivy (existing)
  - ort (existing, for semantic)
---

# SPEC-023: zetl bench — Search Quality Benchmarking Harness

## Information Table

| Field        | Value                                                           |
| ------------ | --------------------------------------------------------------- |
| Document ID  | SPEC-023                                                        |
| Title        | zetl bench — Search Quality Benchmarking Harness                |
| Version      | 0.1.0                                                           |
| Status       | Draft                                                           |
| Author       | Agent (USDD Protocol v1.3.0)                                    |
| Date         | 2026-04-07                                                      |
| Audience     | Agent, Human                                                    |
| Trace        | USDD §2 (Vision -> Specification)                               |
| Parent       | SPEC-002: zetl search — Full-Text Content Search                |
| Related      | SPEC-013: Tantivy Full-Text Search; SPEC-018: Semantic Search   |
| Dependencies | tantivy (existing), ort (existing, for semantic)                |

---

## 1. Overview

### 1.1 Problem

zetl has three search backends — BM25 full-text via Tantivy (SPEC-013), semantic vector search via ONNX embeddings (SPEC-018), and hybrid BM25+vector via reciprocal rank fusion (SPEC-018) — plus graph-scoped search with `--near` (SPEC-013). There is currently no way to measure search quality, compare backends against each other, or detect quality regressions when the codebase changes. A developer who modifies tokenisation, adjusts BM25 parameters, retrains the embedding model, or changes the chunking strategy has no feedback signal beyond manual spot-checking.

### 1.2 Core Insight

Search quality is measurable. Information retrieval has well-established metrics — precision, recall, MRR, nDCG, MAP — that reduce subjective "did the right thing appear?" to numbers that can be tracked, compared, and gated in CI. The missing piece is not a new algorithm but a harness: a way to express "for this query, these are the relevant documents" and then run that expectation against every backend.

### 1.3 Design Philosophy

- **Fixtures are data, not code.** Benchmark suites are JSON or TOML files that any user can author without writing Rust. An agent can generate them from a vault's contents.
- **All backends, one command.** `zetl bench` runs every available search backend against every query in the fixture and reports metrics side by side. No need to invoke each backend separately.
- **Graded relevance.** Real search quality requires distinguishing "highly relevant" from "marginally relevant." Fixtures support relevance grades (0-3), enabling nDCG alongside binary metrics.
- **Regression detection is first-class.** The harness can compare two runs and exit non-zero when metrics drop, making it suitable for CI gating.
- **Output is dual-audience.** Human-readable table output for interactive use; JSON output for agent consumption and downstream tooling.

### 1.4 Scope

**In scope:**

- `zetl bench <fixture>` command that runs benchmark suites
- Fixture file format (JSON and TOML) with queries, relevance judgments, and search mode overrides
- Metrics computation: precision@k, recall@k, MRR, nDCG, MAP
- Graded relevance support for nDCG (relevance grades 0-3)
- Per-query and aggregate metric reporting
- Table output (CLI) and JSON output (agent-friendly)
- Delta comparison between two benchmark runs (`--baseline`)
- CI integration: `--threshold` flag with non-zero exit on regression
- Example fixture for the demo vault shipped with zetl
- Graph-scoped benchmarks via `--near` overrides in fixtures

**Out of scope:**

- Automatic relevance judgment generation (user/agent authors fixtures manually)
- Latency benchmarking (use `zetl search` with `ZETL_DEBUG_RENDER=1` or external tools like `hyperfine`)
- A/B testing across different index configurations in a single run
- Fixture generation from click logs or query logs
- Web UI for benchmark results

---

## 2. User Profiles

### 2.1 User Profile: Search Developer

```
Name:        Priya
Role:        Contributor to zetl; modifying search internals
Goals:       Validate that changes to tokenisation, BM25 parameters,
             or chunking strategy do not degrade search quality;
             compare backends objectively
Constraints: Works locally; wants fast feedback; does not want to
             manually re-test dozens of queries after each change
Workflow:    Edits search code, runs `zetl bench fixtures/search.json`,
             checks that metrics have not regressed
Pain point:  "I changed the tokeniser and I have no idea if search
             got better or worse. I need numbers."
```

### 2.2 User Profile: Vault Curator

```
Name:        Jorge
Role:        Maintains a 3,000-note research vault; uses zetl for
             discovery and navigation
Goals:       Determine which search backend works best for his vault;
             build a regression suite for queries he cares about
Constraints: Not a Rust developer; comfortable with CLI and JSON;
             wants to author fixtures without touching code
Workflow:    Writes a fixture file with his most important queries
             and the pages he expects to find; runs `zetl bench`
             to see which backend serves him best
Pain point:  "Hybrid search sometimes finds things BM25 misses, but
             I don't know if it's consistently better or just lucky
             on a few queries."
```

### 2.3 User Profile: CI Pipeline

```
Name:        (automated)
Role:        Continuous integration runner executing on every push
Goals:       Detect search quality regressions before merge;
             gate PRs on minimum quality thresholds
Constraints: Non-interactive; needs JSON output and exit codes;
             must complete within CI time budget
Workflow:    `zetl index && zetl bench fixtures/regression.json
             --format json --threshold mrr=0.7,ndcg=0.6`
             Exit 0 if all thresholds met; exit 1 otherwise.
Pain point:  "We merged a change that broke hybrid search ranking
             and didn't notice for two weeks."
```

### 2.4 User Profile: Agent Operator

```
Name:        (LLM agent)
Role:        Automated agent that queries zetl search as part of
             a retrieval-augmented pipeline
Goals:       Generate and maintain benchmark fixtures from vault
             content; evaluate search quality programmatically;
             select the best backend per query type
Constraints: Consumes JSON; needs structured metric output;
             may generate fixtures from known-good query/result pairs
Workflow:    Agent authors a fixture, runs `zetl bench --format json`,
             parses metrics, decides whether to use bm25, semantic,
             or hybrid for downstream queries
Pain point:  "I need to know which search mode to use for different
             query types, and I need that answer as structured data."
```

---

## 3. Requirements

### 3.1 Functional Requirements — Benchmark Execution

```
REQ-127: Benchmark Command

The system SHALL provide a `zetl bench <fixture>` command that
reads a benchmark fixture file and executes all queries against
the vault's search backends,
FOR all user roles
WITH the fixture file path as a required positional argument
AND the vault resolved using the same rules as `zetl search`
  (current directory, --vault flag, ZETL_VAULT env var)
AND the search index built lazily if absent (same as SPEC-013
  REQ-013-004).

Trace:
- TEST-150
- CON-037
- ADR-060
```

```
REQ-128: Multi-Backend Execution

The system SHALL, for each query in the fixture, execute the
query against all available search backends: bm25, semantic,
and hybrid,
FOR all user roles
WITH backends determined by compile-time feature flags:
  - bm25: always available (tantivy)
  - semantic: available when compiled with --features semantic
  - hybrid: available when compiled with --features semantic
AND a query-level `modes` override in the fixture that restricts
  which backends are tested for that query
AND backends not available at compile time silently skipped
  (not an error)
AND each backend queried with the same limit (default k=10,
  overridable per-query or globally in the fixture).

Trace:
- TEST-151
- CON-037
```

```
REQ-129: Relevance Metrics Computation

The system SHALL compute the following metrics for each
(query, backend) pair:
  - precision@k: fraction of top-k results that are relevant
  - recall@k: fraction of all relevant documents found in top-k
  - MRR (mean reciprocal rank): 1 / rank of first relevant result
  - nDCG (normalised discounted cumulative gain): graded relevance
    metric using log2 discounting
  - MAP (mean average precision): mean of precision at each
    relevant document's rank position
FOR all user roles
WITH k determined by the fixture's `k` field (default 10)
AND binary relevance derived from graded relevance: grade >= 1
  is relevant, grade 0 is not relevant
AND nDCG using graded relevance values directly (0-3).

Trace:
- TEST-152
- CON-037
```

```
REQ-130: Graded Relevance Support

The system SHALL support graded relevance judgments in fixture
files, with integer grades from 0 to 3:
  - 0: not relevant
  - 1: marginally relevant
  - 2: relevant
  - 3: highly relevant
FOR all user roles
WITH grades used directly for nDCG computation
AND grades >= 1 treated as binary "relevant" for precision,
  recall, MRR, and MAP.

Trace:
- TEST-152
- CON-038
```

```
REQ-131: Aggregate Metrics

The system SHALL compute aggregate metrics across all queries
in the fixture:
  - mean precision@k
  - mean recall@k
  - MRR (mean across queries)
  - mean nDCG
  - MAP (mean across queries)
FOR all user roles
WITH aggregates computed per backend
AND reported alongside per-query metrics.

Trace:
- TEST-153
- CON-037
```

### 3.2 Functional Requirements — Output and Comparison

```
REQ-132: Dual Output Format

The system SHALL support two output formats:
  - table: human-readable aligned table written to stdout
  - json: machine-readable JSON written to stdout
FOR all user roles
WITH the format selected via `--format <table|json>`
  (default: table)
AND the table format showing per-query rows grouped by backend
  with a summary row per backend
AND the JSON format including all per-query metrics, aggregate
  metrics, and fixture metadata.

Trace:
- TEST-154
- CON-037
```

```
REQ-133: Baseline Comparison and Delta Reporting

The system SHALL, when `--baseline <path>` is provided, load
a previous benchmark run (JSON) and compute metric deltas:
  delta = current - baseline
FOR all user roles
WITH deltas displayed alongside current metrics in both table
  and JSON output
AND positive deltas (improvements) indicated with "+" prefix
AND negative deltas (regressions) indicated with "-" prefix
AND queries present in the current run but absent from the
  baseline marked as "new"
AND queries present in the baseline but absent from the current
  run marked as "removed".

Trace:
- TEST-155
- CON-037
```

```
REQ-134: CI Threshold Gating

The system SHALL, when `--threshold <metric=value,...>` is
provided, exit with code 1 if any aggregate metric for any
backend falls below the specified threshold,
FOR CI pipeline users
WITH thresholds specified as comma-separated key=value pairs:
  e.g., `--threshold mrr=0.7,ndcg=0.6,precision=0.5`
AND supported metric keys: precision, recall, mrr, ndcg, map
AND the failing metrics and their values reported to stderr
  before exit
AND exit code 0 if all thresholds are met.

Trace:
- TEST-156
- CON-037
```

### 3.3 Functional Requirements — Fixtures

```
REQ-135: Graph-Scoped Benchmarks

The system SHALL support a `near` field on individual queries
in the fixture file, restricting search to the graph
neighbourhood of the specified anchor page (same semantics as
`--near` in SPEC-013),
FOR all user roles
WITH an optional `depth` field (default 1)
AND relevance judgments evaluated against the scoped result set.

Trace:
- TEST-157
- CON-038
```

```
REQ-136: Example Fixture

The system SHALL ship an example benchmark fixture at
`fixtures/demo-bench.json` that exercises all three backends
against the demo vault,
FOR all user roles
WITH at least 5 queries covering:
  - a keyword query best served by BM25
  - a conceptual query best served by semantic search
  - a hybrid query benefiting from fusion
  - a graph-scoped query with --near
  - a query with graded relevance (grades 0-3)
AND the fixture serving as documentation-by-example for the
  fixture format.

Trace:
- TEST-150
```

---

## 4. Architecture

### 4.1 Architecture Decisions

```
ADR-060: Fixture-Driven Benchmarking over Programmatic Test Suites

Status: Proposed

Context:
  Search quality benchmarks need a set of queries with expected
  results. Two approaches were considered:

  Option A — Rust test functions: Each benchmark is a #[test]
  function that constructs a query, runs it, and asserts on
  metrics. Adding a new benchmark requires writing Rust code,
  recompiling, and understanding the test framework.

  Option B — External fixture files: Benchmarks are data files
  (JSON or TOML) that declare queries and relevance judgments.
  A single `zetl bench` command reads the fixture and runs all
  queries. Adding a new benchmark means editing a JSON file.

Decision:
  Implement Option B (external fixture files).

Rationale:
  - Fixtures are authorable by non-developers, including agents
    and vault curators who understand their queries but not Rust.
  - Fixtures are portable: the same file works across machines,
    CI environments, and zetl versions.
  - Fixtures separate the "what to measure" concern from the
    "how to measure" concern. The harness code changes rarely;
    the fixtures change frequently as the vault evolves.
  - Fixtures can be generated programmatically by agents.
  - Rust tests are still used for unit-testing the metric
    computation functions themselves.

Consequences:
  + Non-developers can author and maintain benchmarks
  + Fixtures are version-controllable alongside the vault
  + Single harness implementation serves all benchmarks
  + Agents can generate fixtures as structured data
  - Fixture format must be documented and validated
  - Fixture files can become stale if the vault changes
    (mitigated by clear error messages on missing pages)
```

```
ADR-061: JSON and TOML Dual Fixture Format

Status: Proposed

Context:
  Fixture files need a serialization format. Candidates:

  Option A — JSON only: Universal, agent-friendly, but verbose
  and comment-hostile. Relevance judgments with inline notes
  are awkward.

  Option B — TOML only: Human-friendly with comments, but
  nested arrays of objects are syntactically heavy. Agent
  generation is less natural than JSON.

  Option C — Both: Accept either format, detected by file
  extension (.json or .toml). Each format maps to the same
  internal schema.

Decision:
  Implement Option C (both JSON and TOML).

Rationale:
  - JSON is the natural output of agents and the natural input
    for `--baseline` comparison (benchmark results are JSON).
  - TOML is the natural authoring format for humans who want
    to annotate fixtures with comments explaining relevance
    judgments.
  - Detection by file extension is trivial and unambiguous.
  - The internal schema is the same; only deserialization
    differs.

Consequences:
  + Agents produce JSON fixtures naturally
  + Humans author TOML fixtures with comments
  + Fixture schema is format-agnostic
  - Two parsers to maintain (mitigated by serde — same
    derive, different deserializer)
  - Documentation must show examples in both formats
```

### 4.2 Component Integration

```
                     ┌────────────────┐
                     │   zetl bench   │
                     │   (command)    │
                     └───────┬────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
       ┌──────▼──────┐ ┌────▼─────┐ ┌──────▼──────┐
       │   Fixture   │ │  Search  │ │   Metric    │
       │   Loader    │ │  Runner  │ │   Engine    │
       │             │ │          │ │             │
       │ - parse JSON│ │ - bm25   │ │ - precision │
       │ - parse TOML│ │ - semantic│ │ - recall   │
       │ - validate  │ │ - hybrid │ │ - MRR      │
       └──────┬──────┘ │ - scoped │ │ - nDCG     │
              │        └────┬─────┘ │ - MAP      │
              │             │       └──────┬──────┘
              │             │              │
       ┌──────▼─────────────▼──────────────▼──────┐
       │              Report Builder              │
       │                                          │
       │  - per-query metrics per backend         │
       │  - aggregate metrics per backend         │
       │  - delta computation (--baseline)        │
       │  - threshold checking (--threshold)      │
       │  - table formatter                       │
       │  - JSON serializer                       │
       └──────────────────────────────────────────┘
              │                    │
              ▼                    ▼
         table (stdout)      JSON (stdout)
```

**Integration points:**

1. **Fixture Loader -> Search Runner.** The loader parses the fixture file, validates its schema, and produces a `Vec<BenchQuery>`. The search runner iterates over each query and dispatches to the appropriate backends.

2. **Search Runner -> Existing Search Infrastructure.** The search runner calls the same search functions used by `zetl search` — Tantivy BM25 queries (SPEC-013), semantic vector queries (SPEC-018), and hybrid reciprocal rank fusion (SPEC-018). No new search code is written; the harness is a consumer of existing search APIs.

3. **Search Runner -> Metric Engine.** For each (query, backend) pair, the search runner passes the result list and the relevance judgments to the metric engine, which computes all metrics.

4. **Metric Engine -> Report Builder.** Per-query and aggregate metrics are collected into a `BenchReport` struct, which the report builder formats for output.

### 4.3 Data Model

```rust
/// A single query in a benchmark fixture.
struct BenchQuery {
    /// Unique identifier for the query within the fixture.
    id: String,
    /// The search query text.
    query: String,
    /// Expected relevant documents with relevance grades.
    /// Key: page name (same resolution as zetl search).
    /// Value: relevance grade (0-3).
    relevant: HashMap<String, u8>,
    /// Number of results to evaluate (default: 10).
    k: Option<usize>,
    /// Restrict to specific backends for this query.
    /// None = run all available backends.
    modes: Option<Vec<SearchMode>>,
    /// Graph-scoped search anchor page.
    near: Option<String>,
    /// Graph scope depth (requires near).
    depth: Option<usize>,
}

/// Search backend mode.
enum SearchMode {
    Bm25,
    Semantic,
    Hybrid,
}

/// Top-level benchmark fixture.
struct BenchFixture {
    /// Human-readable name for the fixture.
    name: String,
    /// Optional description.
    description: Option<String>,
    /// Default k for all queries (overridable per-query).
    default_k: Option<usize>,
    /// The queries in the fixture.
    queries: Vec<BenchQuery>,
}

/// Metrics for a single (query, backend) pair.
struct QueryMetrics {
    query_id: String,
    backend: SearchMode,
    precision_at_k: f64,
    recall_at_k: f64,
    mrr: f64,
    ndcg: f64,
    average_precision: f64,
    k: usize,
    /// Number of results returned by the backend.
    results_returned: usize,
    /// Number of relevant documents in the judgment set.
    total_relevant: usize,
}

/// Aggregate metrics for a single backend across all queries.
struct AggregateMetrics {
    backend: SearchMode,
    mean_precision_at_k: f64,
    mean_recall_at_k: f64,
    mrr: f64,
    mean_ndcg: f64,
    map: f64,
    queries_evaluated: usize,
}

/// Complete benchmark report.
struct BenchReport {
    fixture_name: String,
    timestamp: String,
    vault_path: String,
    per_query: Vec<QueryMetrics>,
    aggregates: Vec<AggregateMetrics>,
}
```

### 4.4 Metric Computation

**Precision@k:**

```
precision@k = |{d in top-k : d is relevant}| / k
```

A document is relevant if its grade >= 1.

**Recall@k:**

```
recall@k = |{d in top-k : d is relevant}| / |{all relevant documents}|
```

If no documents are marked relevant, recall is defined as 1.0 (vacuous truth).

**MRR (Mean Reciprocal Rank):**

```
MRR = 1 / rank_of_first_relevant_result
```

If no relevant result appears in top-k, MRR is 0.0 for that query.

**nDCG (Normalised Discounted Cumulative Gain):**

```
DCG@k  = sum_{i=1}^{k} (2^{rel_i} - 1) / log2(i + 1)
IDCG@k = DCG@k computed on the ideal ranking (grades sorted descending)
nDCG@k = DCG@k / IDCG@k
```

Uses graded relevance (0-3) directly. If IDCG is 0.0 (no relevant documents), nDCG is defined as 1.0.

**MAP (Mean Average Precision):**

```
AP = (1 / |relevant|) * sum_{k=1}^{n} (precision@k * rel_k)
```

Where `rel_k` is 1 if the document at rank k is relevant, 0 otherwise. MAP is the mean of AP across queries.

### 4.5 Query Flow

1. Parse the fixture file (JSON or TOML) into `BenchFixture`.
2. Validate: all query IDs unique, grades in 0-3, modes valid.
3. Ensure search index exists (build lazily if needed).
4. If `--features semantic` is enabled, ensure vector index exists.
5. For each query in the fixture:
   a. Determine active backends (fixture `modes` override, or all available).
   b. For each active backend:
      - Execute the query using the corresponding search function.
      - If `near` is set, apply graph scoping.
      - Extract the top-k result page names.
      - Compute all metrics against the relevance judgments.
      - Store `QueryMetrics`.
6. Compute `AggregateMetrics` per backend.
7. If `--baseline` is provided, load the baseline `BenchReport` and compute deltas.
8. If `--threshold` is provided, check aggregate metrics and set exit code.
9. Format and output the report.

### 4.6 Table Output Format

```
zetl bench fixtures/demo-bench.json

Fixture: demo-vault-search (5 queries)
Vault:   /home/user/demo-vault

Per-query results:
 Query              | Backend  | P@10  | R@10  | MRR   | nDCG  | MAP
--------------------+----------+-------+-------+-------+-------+------
 algorithm basics   | bm25     | 0.400 | 0.667 | 1.000 | 0.712 | 0.622
 algorithm basics   | semantic | 0.300 | 0.500 | 0.500 | 0.583 | 0.467
 algorithm basics   | hybrid   | 0.500 | 0.833 | 1.000 | 0.801 | 0.733
 feedback systems   | bm25     | 0.200 | 0.333 | 0.333 | 0.401 | 0.289
 feedback systems   | semantic | 0.400 | 0.667 | 1.000 | 0.698 | 0.611
 feedback systems   | hybrid   | 0.400 | 0.667 | 1.000 | 0.723 | 0.644
 ...

Aggregates:
 Backend  | P@10  | R@10  | MRR   | nDCG  | MAP   | Queries
----------+-------+-------+-------+-------+-------+--------
 bm25     | 0.340 | 0.560 | 0.733 | 0.612 | 0.534 | 5
 semantic | 0.380 | 0.620 | 0.800 | 0.668 | 0.578 | 5
 hybrid   | 0.440 | 0.740 | 0.900 | 0.751 | 0.672 | 5
```

### 4.7 Delta Output Format (with --baseline)

```
Aggregates (vs baseline 2026-04-06):
 Backend  | P@10         | R@10         | MRR          | nDCG         | MAP
----------+--------------+--------------+--------------+--------------+----------
 bm25     | 0.340        | 0.560        | 0.733        | 0.612        | 0.534
          |              |              |              |              |
 semantic | 0.380 +0.040 | 0.620 +0.080 | 0.800        | 0.668 +0.031 | 0.578
          |              |              |              |              |
 hybrid   | 0.440 +0.020 | 0.740 +0.040 | 0.900        | 0.751 -0.012 | 0.672
```

---

## 5. Contract Specifications

```
CON-037: bench (search quality benchmarking)

zetl bench <FIXTURE> [OPTIONS]

Arguments:
  <FIXTURE>            Path to a benchmark fixture file (.json or .toml)

Options:
  --vault <PATH>       Vault root directory [default: current directory]
  --format <FORMAT>    Output format: table or json [default: table]
  --baseline <PATH>    Path to a previous benchmark run (JSON) for delta
                       comparison
  --threshold <SPEC>   Comma-separated metric=value pairs for CI gating
                       (e.g., mrr=0.7,ndcg=0.6,precision=0.5)
  --k <N>              Override default k for all queries [default: 10]

Exit codes:
  0  Benchmark completed successfully; all thresholds met (if specified)
  1  One or more thresholds not met (only with --threshold)
  2  Invalid fixture, missing vault, or other error

JSON output schema (--format json):
{
  "fixture_name": "demo-vault-search",
  "timestamp": "2026-04-07T14:30:00Z",
  "vault_path": "/home/user/demo-vault",
  "per_query": [
    {
      "query_id": "algo-basics",
      "query": "algorithm basics",
      "backend": "bm25",
      "k": 10,
      "precision_at_k": 0.400,
      "recall_at_k": 0.667,
      "mrr": 1.000,
      "ndcg": 0.712,
      "map": 0.622,
      "results_returned": 10,
      "total_relevant": 3
    }
  ],
  "aggregates": [
    {
      "backend": "bm25",
      "mean_precision_at_k": 0.340,
      "mean_recall_at_k": 0.560,
      "mrr": 0.733,
      "mean_ndcg": 0.612,
      "map": 0.534,
      "queries_evaluated": 5
    }
  ],
  "thresholds": {
    "passed": true,
    "checks": [
      { "metric": "mrr", "required": 0.7, "actual": { "bm25": 0.733, "semantic": 0.800, "hybrid": 0.900 }, "passed": true }
    ]
  },
  "baseline_deltas": null
}

Table output:
  See section 4.6 and 4.7.

Implements:
- REQ-127, REQ-128, REQ-129, REQ-131, REQ-132, REQ-133, REQ-134

Verified by:
- TEST-150, TEST-151, TEST-152, TEST-153, TEST-154, TEST-155, TEST-156
```

```
CON-038: Benchmark Fixture Format

JSON fixture schema:
{
  "name": "demo-vault-search",
  "description": "Benchmark suite for the zetl demo vault",
  "default_k": 10,
  "queries": [
    {
      "id": "algo-basics",
      "query": "algorithm basics",
      "relevant": {
        "Algorithm": 3,
        "Data Structures": 2,
        "Sorting": 1
      },
      "k": 10,
      "modes": ["bm25", "hybrid"],
      "near": null,
      "depth": null
    },
    {
      "id": "feedback-conceptual",
      "query": "feedback loops in complex systems",
      "relevant": {
        "Cybernetics": 3,
        "PID Controllers": 2,
        "Homeostasis": 2,
        "Control Theory": 1
      },
      "modes": ["semantic", "hybrid"],
      "near": "Systems Theory",
      "depth": 2
    }
  ]
}

TOML fixture equivalent:
  name = "demo-vault-search"
  description = "Benchmark suite for the zetl demo vault"
  default_k = 10

  [[queries]]
  id = "algo-basics"
  query = "algorithm basics"
  k = 10
  modes = ["bm25", "hybrid"]

  [queries.relevant]
  Algorithm = 3
  "Data Structures" = 2
  Sorting = 1

  [[queries]]
  id = "feedback-conceptual"
  query = "feedback loops in complex systems"
  modes = ["semantic", "hybrid"]
  near = "Systems Theory"
  depth = 2

  # Cybernetics is the most relevant — it directly discusses
  # feedback loops in the context of system regulation
  [queries.relevant]
  Cybernetics = 3
  "PID Controllers" = 2
  Homeostasis = 2
  "Control Theory" = 1

Field specifications:
  name          (required) String. Human-readable fixture name.
  description   (optional) String. Description of the fixture.
  default_k     (optional) Integer >= 1. Default k for all queries.
                Defaults to 10 if absent.
  queries       (required) Array of query objects. Must be non-empty.

Query object fields:
  id            (required) String. Unique within the fixture.
  query         (required) String. The search query text.
  relevant      (required) Map of page_name -> grade (integer 0-3).
                At least one entry with grade >= 1 is required.
  k             (optional) Integer >= 1. Overrides default_k.
  modes         (optional) Array of "bm25", "semantic", "hybrid".
                Null or absent = all available backends.
  near          (optional) String. Anchor page for graph-scoped search.
  depth         (optional) Integer >= 1. Graph scope depth.
                Requires near. Defaults to 1 if near is set.

Validation rules:
  - All query IDs must be unique.
  - All relevance grades must be integers in [0, 3].
  - Each query must have at least one relevant document (grade >= 1).
  - modes values must be one of: "bm25", "semantic", "hybrid".
  - depth without near is an error.
  - k must be >= 1.
  - Duplicate page names within a single query's relevant map
    are an error (caught by JSON/TOML parsers naturally).

Implements:
- REQ-130, REQ-135, REQ-136

Verified by:
- TEST-150, TEST-152, TEST-157
```

---

## 6. Non-Functional Requirements

```
NFR-049: Benchmark Execution Performance

A benchmark suite of 50 queries against a vault of 10,000 indexed
documents SHALL complete in <= 30 seconds UNDER single-threaded
execution on commodity hardware WITH 95th percentile.

Rationale:
  Each query is a Tantivy search (< 100ms per SPEC-013 NFR-013-002)
  plus metric computation (microseconds). 50 queries x 3 backends
  = 150 searches x 100ms = 15 seconds. The 30-second budget
  provides 2x headroom for semantic queries, fixture parsing,
  and report generation.
```

```
NFR-050: Metric Computation Accuracy

All metric computations SHALL produce results accurate to
4 decimal places, matching reference implementations (trec_eval
for MAP/MRR, standard nDCG formulas with log2 discounting).

Rationale:
  Benchmark metrics are compared across runs. Floating-point
  inconsistencies between runs would produce false regressions.
  4 decimal places is the standard reporting precision in IR
  evaluation and matches the output format.
```

---

## 7. Test Specifications

### 7.1 Benchmark Execution Tests

```
TEST-150: Benchmark Command Execution

Scenario: Basic benchmark run
Given: A vault with 10 indexed Markdown files
And: A fixture file with 3 queries and relevance judgments
When: `zetl bench fixture.json` is run
Then:
  - Exit code 0
  - Stdout contains a table with per-query metrics
  - Stdout contains an aggregates section
  - All metric values are in [0.0, 1.0]

Scenario: Fixture file not found
When: `zetl bench nonexistent.json` is run
Then: Exit code 2, stderr reports file not found

Scenario: Invalid fixture format
Given: A file with invalid JSON
When: `zetl bench invalid.json` is run
Then: Exit code 2, stderr reports parse error

Scenario: Example fixture runs against demo vault
Given: The demo vault is indexed
When: `zetl bench fixtures/demo-bench.json` is run
Then: Exit code 0, metrics are computed for all queries

Verifies: REQ-127, REQ-136
```

```
TEST-151: Multi-Backend Execution

Scenario: All backends executed
Given: A vault indexed with both tantivy and semantic features
And: A fixture with a query that has no modes restriction
When: `zetl bench fixture.json` is run
Then: Results include metrics for bm25, semantic, and hybrid

Scenario: Modes override restricts backends
Given: A fixture with a query that has modes: ["bm25"]
When: `zetl bench fixture.json` is run
Then: Results for that query include only bm25 metrics

Scenario: Unavailable backend silently skipped
Given: Compiled without --features semantic
And: A fixture with modes: ["bm25", "semantic"]
When: `zetl bench fixture.json` is run
Then: Results include only bm25 metrics, no error

Verifies: REQ-128
```

```
TEST-152: Metric Computation

Scenario: Perfect ranking
Given: A query with relevant docs A (grade 3), B (grade 2)
And: Backend returns [A, B, C, D, E] (k=5)
Then:
  - precision@5 = 0.4 (2 relevant out of 5)
  - recall@5 = 1.0 (both relevant found)
  - MRR = 1.0 (first result is relevant)
  - nDCG > 0.9 (ideal order)
  - MAP = (1/2) * (1.0 + 1.0) = 1.0

Scenario: No relevant results in top-k
Given: A query with relevant docs [A, B]
And: Backend returns [C, D, E, F, G] (k=5)
Then:
  - precision@5 = 0.0
  - recall@5 = 0.0
  - MRR = 0.0
  - nDCG = 0.0
  - MAP = 0.0

Scenario: Graded relevance affects nDCG
Given: Query with A (grade 3), B (grade 1)
And: Backend returns [B, A, C] (k=3)
Then:
  - nDCG < 1.0 (suboptimal order: grade-1 before grade-3)
  - nDCG computed with (2^grade - 1) / log2(rank + 1)

Scenario: All binary metrics treat grade >= 1 as relevant
Given: Query with A (grade 0), B (grade 1), C (grade 2)
And: Backend returns [A, B, C] (k=3)
Then:
  - precision@3 = 0.667 (B and C are relevant)
  - A is not counted as relevant despite being in judgments

Verifies: REQ-129, REQ-130
```

### 7.2 Output and Comparison Tests

```
TEST-153: Aggregate Metrics

Scenario: Aggregates computed correctly
Given: A fixture with 3 queries
And: All queries evaluated against bm25
When: Benchmark completes
Then:
  - Aggregate mean_precision_at_k = mean of 3 per-query precision values
  - Aggregate mrr = mean of 3 per-query MRR values
  - Aggregate map = mean of 3 per-query AP values
  - All aggregates reported per backend

Verifies: REQ-131
```

```
TEST-154: Output Formats

Scenario: Table output
When: `zetl bench fixture.json --format table` is run
Then:
  - Stdout contains aligned columns with headers
  - Per-query rows are present
  - Aggregate summary rows are present

Scenario: JSON output
When: `zetl bench fixture.json --format json` is run
Then:
  - Stdout is valid JSON
  - JSON contains "fixture_name", "per_query", "aggregates"
  - All metric values are numbers

Scenario: JSON output is parseable by agents
When: `zetl bench fixture.json --format json` output is parsed
Then:
  - per_query is an array of objects with query_id, backend, and metrics
  - aggregates is an array of objects with backend and metrics

Verifies: REQ-132
```

```
TEST-155: Baseline Comparison

Scenario: Delta computation
Given: A baseline JSON from a previous run with bm25 MRR = 0.700
And: Current run produces bm25 MRR = 0.733
When: `zetl bench fixture.json --baseline baseline.json` is run
Then:
  - Output shows MRR delta of +0.033
  - Table format shows "+0.033" next to the metric

Scenario: New query in current run
Given: Baseline has queries [A, B], current fixture has [A, B, C]
When: Compared with --baseline
Then: Query C is marked as "new" in the report

Scenario: Removed query
Given: Baseline has queries [A, B, C], current fixture has [A, B]
When: Compared with --baseline
Then: Query C is marked as "removed" in the report

Verifies: REQ-133
```

```
TEST-156: CI Threshold Gating

Scenario: All thresholds met
Given: Benchmark produces bm25 MRR = 0.733, nDCG = 0.612
When: `zetl bench fixture.json --threshold mrr=0.7,ndcg=0.6`
Then: Exit code 0

Scenario: Threshold not met
Given: Benchmark produces bm25 MRR = 0.650
When: `zetl bench fixture.json --threshold mrr=0.7`
Then:
  - Exit code 1
  - Stderr reports: "Threshold failed: bm25 mrr = 0.650 < 0.700"

Scenario: Multiple thresholds, one fails
Given: Benchmark produces MRR = 0.800, nDCG = 0.500
When: `zetl bench fixture.json --threshold mrr=0.7,ndcg=0.6`
Then:
  - Exit code 1
  - Stderr reports the failing nDCG threshold

Scenario: Threshold checked per backend
Given: bm25 MRR = 0.800, semantic MRR = 0.650
When: `zetl bench fixture.json --threshold mrr=0.7`
Then:
  - Exit code 1
  - Stderr reports semantic backend failing MRR threshold

Verifies: REQ-134
```

### 7.3 Fixture and Graph-Scoped Tests

```
TEST-157: Graph-Scoped Benchmarks

Scenario: Near field restricts search scope
Given: A fixture with a query that has near: "Systems Theory", depth: 2
And: The vault has Systems Theory linked to Cybernetics and Control Theory
When: Benchmark runs that query
Then:
  - Search results are restricted to the neighbourhood
  - Metrics are computed against the scoped result set

Scenario: Near page resolution
Given: A fixture with near: "nonexistent page"
When: Benchmark runs that query
Then:
  - Query is reported as errored (not a fixture-level failure)
  - Other queries in the fixture still execute

Scenario: Depth without near is rejected
Given: A fixture with a query that has depth: 2 but no near field
When: `zetl bench fixture.json` is run
Then: Exit code 2, validation error reported

Verifies: REQ-135
```
