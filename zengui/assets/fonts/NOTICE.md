# Bundled fonts (#533)

zengui embeds these faces with `include_bytes!` (`zengui/src/view/fonts.rs`),
so every host draws the same type. They are third-party works under their own
licences; zengui's Apache-2.0 does not apply to them.

| File | Work | Version | Licence | Source |
|---|---|---|---|---|
| `Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf` | Inter, by The Inter Project Authors | 4.1 (`extras/ttf`) | SIL OFL 1.1 — `OFL-Inter.txt` | <https://github.com/rsms/inter/releases/tag/v4.1> |
| `JetBrainsMonoNL-Regular.ttf` | JetBrains Mono NL, by The JetBrains Mono Project Authors | 2.304 (`fonts/ttf`) | SIL OFL 1.1 — `OFL-JetBrainsMono.txt` | <https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304> |
| `lucide.ttf` | Lucide icons, by Lucide Contributors — **subset** to the names in `lucide-icons.txt` by `scripts/subset-icons.py` | lucide-static 1.52.0 (`font/lucide.ttf`) | ISC — `LICENSE-lucide.txt` | <https://registry.npmjs.org/lucide-static/-/lucide-static-1.52.0.tgz> |

The Inter and JetBrains Mono files are unmodified. The Lucide file is a
modified version (glyph subset), as the ISC licence permits.

```
40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82  Inter-Regular.ttf
97ad806f526e41546d46365bb3a393145f75b7b1568913db74549ad8b8dba872  Inter-Medium.ttf
78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3  Inter-SemiBold.ttf
fb3b2575d7b0657359707993288f12a7360344d39387bb26050e276d61f6bd2a  JetBrainsMonoNL-Regular.ttf
121a0408e7f819a67d05cfe6c1bb7c748cb3c6cd1c8462a738e859fda3a4caa0  lucide.ttf
```
