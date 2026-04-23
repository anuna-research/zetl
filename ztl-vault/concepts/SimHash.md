# SimHash

SimHash is a locality-sensitive hashing technique that produces similar fingerprints for similar inputs. ztl uses it for fuzzy page name matching via the [[Similar Command]].

## The idea

Traditional hashes (like SHA-256 or BLAKE3) are designed so that even tiny input changes produce completely different outputs. SimHash does the opposite — similar inputs produce fingerprints that differ in only a few bits.

## How it works

1. Break the input into overlapping pieces (character trigrams for page names)
2. Hash each piece independently
3. Combine the hashes by voting: for each bit position, count +1 for 1-bits and -1 for 0-bits across all piece hashes
4. The final fingerprint bit is 1 if the vote is positive, 0 otherwise

## Hamming distance

Similarity is measured by Hamming distance — the number of differing bits between two 64-bit fingerprints. Lower distance means more similar:

| Distance | Interpretation |
|----------|---------------|
| 0 | Identical (after normalization) |
| 1–5 | Very similar (minor typos) |
| 6–12 | Somewhat similar |
| 13+ | Likely different |

Comparing two fingerprints takes a single XOR + popcount — O(1).

## Use cases in ztl

- **Typo correction:** `ztl similar "zettelkasen"` finds "Zettelkasten"
- **Duplicate detection:** pages with very low Hamming distance might be candidates for merging
- **Fuzzy lookup:** when `--fuzzy` is passed to [[Links Command]], page names are matched via SimHash

## Limitations

SimHash is effective for short strings (page names) but not for full document content. For content search, use [[Search Command]]. For content-level change detection, ztl uses [[Merkle Tree]] hashing instead.

For the implementation details, see [[architecture/SimHash]].

See also: [[architecture/SimHash]], [[Similar Command]], [[Search Command]]
