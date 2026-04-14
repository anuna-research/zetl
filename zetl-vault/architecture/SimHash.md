# SimHash

SimHash is the fuzzy matching algorithm behind `zetl similar` — see [[Similar Command]]. For the conceptual explanation, see [[concepts/SimHash]].

## Implementation

1. **Tokenize** — split the page name into overlapping character trigrams (3-character windows)
2. **Hash** — compute a hash for each trigram
3. **Aggregate** — for each bit position in the 64-bit fingerprint, sum the trigram hash bits (+1 for 1, -1 for 0). The final bit is 1 if the sum is positive, 0 otherwise.
4. **Compare** — Hamming distance between two 64-bit fingerprints counts the differing bits

## Hamming distance

Lower distance means more similar names:

| Distance | Interpretation |
|----------|---------------|
| 0 | Identical (after normalization) |
| 1–5 | Very similar (minor typos) |
| 6–12 | Somewhat similar (default threshold) |
| 13+ | Likely different pages |

The default threshold of 12 is tunable via `--threshold`.

## Performance

SimHash comparison is O(1) per pair (a single XOR + popcount). The initial fingerprint computation is O(n) in page name length. For a vault with 10,000 pages, comparing a query against all pages takes microseconds.

## Limitations

SimHash works well for page names but is not suitable for content similarity. For content search, use [[Search Command]]. For semantic similarity, future work may explore embedding-based approaches.

See also: [[concepts/SimHash]], [[Similar Command]], [[SPEC-001 Link Graph CLI]]
