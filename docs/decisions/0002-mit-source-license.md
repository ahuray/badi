# ADR 0002: license Badi under MIT

- Status: accepted
- Date: 2026-08-31
- Scope: original source code and documentation in the Badi repository

## Context

Badi needs explicit redistribution terms before public review, contribution,
packaging, or release. The owner requested the same MIT License used by
[Omarchy](https://github.com/omacom/omarchy/blob/quattro/LICENSE).

The source license is only one part of Badi's provenance boundary. Model
weights, tokenizers, evaluation corpora, generated artifacts, dependencies,
and copied third-party material retain their own licenses and review
requirements.

## Decision

Badi's original source code and documentation are licensed under the MIT
License. The normative terms are the repository's top-level
[`LICENSE`](../../LICENSE), with copyright held by Ahura Arj.

Package metadata uses the SPDX identifier `MIT`. Public package-name and
trademark clearance remain separate decisions; choosing MIT does not resolve
the unrelated `badi` command-name collision.

## Consequences

- People may use, copy, modify, merge, publish, distribute, sublicense, and
  sell covered Badi material while preserving the copyright and permission
  notice.
- Badi is provided without warranty under the terms in `LICENSE`.
- Contributions require an explicit contribution policy before accepting
  broad external submissions; MIT alone does not define that workflow.
- No model artifact is relicensed by this decision. Every model remains bound
  to its pinned upstream license and Badi's separate runtime/evaluation gate.
- Historical audit reports keep their original pre-license findings as
  snapshot evidence. Current documentation records this finding as resolved.

## Alternatives rejected

### Leave the repository unlicensed

Rejected. Public visibility does not grant reuse rights and would keep an
already identified handoff and release blocker open.

### Copy Omarchy's copyright notice

Rejected. Badi uses the same MIT terms, not Omarchy's authorship. The copyright
notice must identify Badi's owner.

### Apply the source license to model weights and datasets

Rejected. Those are separate works with independent provenance and license
obligations.
