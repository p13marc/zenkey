# `redundancy.v2`'s fixtures

The fixtures of [`../../v2.md`](../../v2.md). Each file states its format
in its `description`. The rules of `redundancy.v1` that this major keeps
read v1's fixtures, one directory up: `elections.json`, `heals.json`,
`lifecycle.json`, `polls.json` and `conclusions.json`.

- `declarations.json`: §2.2, a contract's declaration, and an address's
  governance from its candidates' revisions.
- `descriptors.json`: §2.10 and §2.14, what a follower reads of a
  descriptor: its run, and the governed interface's mark.
- `fences.json`: §2.10 and §2.17, the follower's rule over one sample,
  the hold for one read over a sequence, and a candidate's draw.

Every expectation was computed twice, by two scripts of the text's
drafter that share no code. The first read the text into code and wrote
the files. The second was written from the finished text, and computed
each expectation again from the files' inputs. The two agree on every
case. Both are kept out of the repository, and no file here was blessed.
