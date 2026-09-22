# English completion lexicon

`en.txt` is a modified derivative of the SCOWL American English size-60
Hunspell dictionary, version 2020.12.07. It provides a conservative membership
check for English words formed by joining a generated suffix to an existing
partial word. It supplies neither prediction text nor frequency rankings.
Dictionary membership alone does not establish that a continuation is useful
in its sentence. Names, technical terms, possessives, hyphenated words, and
other valid vocabulary can be absent; absence should cause abstention.

The original dictionary, affixes, and complete license notices are preserved
byte for byte from
[wooorm/dictionaries commit 8cfea406b505e4d7df52d5a19bce525df98c54ab](https://github.com/wooorm/dictionaries/tree/8cfea406b505e4d7df52d5a19bce525df98c54ab/dictionaries/en).
The upstream license identifies the SCOWL command as
`mk-list --accents=strip en_US 60`. The repository commit was resolved through
the GitHub commits API; no floating branch is used during regeneration.
[manifest.json](manifest.json) records immutable source URLs, source and output
SHA-256 digests, counts, generator identity, and expansion version.

The modified list retains only originally lowercase ASCII alphabetic stems,
expands their original affixes with **Hunspell 1.7.3 `unmunch`**, and retains
only lowercase alphabetic results. It excludes roots carrying `NOSUGGEST`,
`FORBIDDENWORD`, `ONLYINCOMPOUND`, `NEEDAFFIX`, or `PSEUDOROOT` flags. The pinned
affix file declares only `NOSUGGEST !` and `ONLYINCOMPOUND c` from that set.
The generator also subtracts all expanded excluded families, preventing an
alternate root or affix combination from reintroducing the same surface word.
Capitalized names, acronyms, numbers, apostrophes, and punctuation are omitted.
No words are manually added or selected from evaluation answers.

This explicit filtering matters because
[Hunspell's unmunch implementation](https://github.com/hunspell/hunspell/blob/v1.7.3/src/tools/unmunch.cxx)
expands prefix/suffix rules without enforcing suggestion or compound flags.
The pinned dictionary uses ASCII stems, single-byte flags, and no continuation
flags. The generator rejects unreviewed affix directives and continuation
syntax. It sorts unique UTF-8 lines in byte order with LF terminators; ASCII
output also has the same Unicode scalar order.

From the repository root:

```sh
# Portable integrity/format check: Python standard library only, no network.
python3 scripts/build-writing-lexicon.py --verify
python3 broker/data/writing-lexicon/test_generator.py

# Exact offline reproduction: build-time Hunspell 1.7.3 and unmunch required.
python3 scripts/build-writing-lexicon.py --check

# Regenerate after an intentional, reviewed generator change.
python3 scripts/build-writing-lexicon.py
```

The build tools are not runtime dependencies. `--check` writes only temporary
files and requires the regenerated list and manifest to match byte for byte.
`--verify` checks bundled source checksums, generator identity, output checksum,
count, ordering, uniqueness, encoding, and alphabetic format without expanding
affixes. It does not replace the stronger `--check` reproduction lane.
Generation was verified using Arch's Hunspell 1.7.3 `unmunch` executable,
SHA-256 `3e774b2c03299240ffca37456b5f68938050f07b053c952d42d1e5fd50cf9a1c`;
the manifest is platform independent and does not require that executable hash.

The generated list has 77,928 words (738,199 bytes). Common words including
`document`, `automatically`, and `address` are present; the malformed forms
`documnet` and `omarchythm` are absent. These are validation examples, not
generator exceptions.

## Distribution notices

Retain the complete [LICENSE](LICENSE) and this modification notice alongside
source distributions and binary distributions embedding `en.txt`. The upstream
notices include SCOWL, Ispell, WordNet, and the other credited sources. This
directory's filtering and generated output are modifications by the Badi
project; the original `.dic`, `.aff`, and license bytes are unchanged. Do not
present the generated list as an unmodified upstream dictionary or use the
upstream contributors' names to endorse Badi.
