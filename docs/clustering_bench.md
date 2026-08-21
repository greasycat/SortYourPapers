# Clustering bench

Measured comparison of ways to group papers, run against the curated set's
abstracts and its arXiv labels. All numbers below come from
`cargo test -p syp-library curated_set -- --ignored --nocapture`, which needs
no API key and no PDFs.

Read **ARI** (adjusted rand index). It is the only measure here corrected for
chance, so it does not reward splitting. The others are reported to show *how*
a strategy fails, not to rank strategies.

## What the text alone supports

60 curated papers against their 4 broad categories:

| strategy | folders | ARI | v-measure |
| --- | --- | --- | --- |
| everything in one folder | 1 | 0.000 | 0.000 |
| one folder per paper | 60 | 0.000 | 0.506 |
| abstract terms, k = labels | 4 | 0.110 | 0.218 |
| pipeline-reduced terms, k = labels | 4 | **0.204** | 0.274 |
| title terms, k = labels | 4 | 0.000 | 0.087 |
| title + abstract, k = labels | 4 | 0.050 | 0.132 |
| words + word pairs, k = labels | 4 | 0.034 | 0.163 |
| latent topics (8), k = labels | 4 | 0.067 | 0.162 |
| latent topics (16), k = labels | 4 | 0.158 | 0.287 |
| 12 rarest terms, k = labels | 4 | 0.004 | 0.090 |
| title + abstract, k unknown | 52 | 0.028 | 0.516 |

Excluding the `Others` category — a grab-bag with no shared subject, so no
method can group it — leaves 45 papers over 3 fields, where the best term
strategy reaches ARI 0.150.

## Does a semantic representation help?

Latent topics — truncated SVD over the same term matrix, so papers sharing
vocabulary only *indirectly* still come out close — is the offline stand-in for
an embedding. On the 3 coherent fields it produced the single best number in
the whole bench, ARI 0.293 at 2 topics, against 0.150 for the best surface-term
method.

That number does not survive resampling. Re-scoring on five
leave-one-fifth-out folds:

| strategy | min | median | max |
| --- | --- | --- | --- |
| title + abstract terms | -0.010 | 0.160 | 0.243 |
| latent topics (2) | 0.116 | 0.164 | 0.204 |
| latent topics (16) | -0.003 | 0.119 | 0.272 |

The spreads overlap almost entirely and the medians are 0.160, 0.164 and 0.119
— indistinguishable. The 0.293 was a favourable split, not a better method.

**So the honest answer to "which approach is best" is: this corpus cannot
tell.** Single-run differences of ±0.1 ARI are within fold-to-fold noise at 45
papers, which is larger than every gap the bench measured. Any ranking claimed
from one run of this set — including one favouring embeddings — would be
reading noise.

The one durable difference is spread, not accuracy: latent topics at 2
components varies least across folds (0.116–0.204), where the term methods
swing through zero. A method that is merely stable is still worth something,
but it is not the same as a method that is better.

## Findings

**Term-overlap clustering is a weak floor, not a candidate.** Even handed the
true number of labels, no lexical strategy passes ARI 0.21, and most sit near
0.1. Replacing the model's labelling with cheap term clustering would be a
regression, not a simplification.

**Smarter lexical features do not rescue it.** Adding adjacent word pairs
scored 0.034 against 0.050 for single words, and keeping only each paper's
twelve rarest terms scored 0.004. Both are worse than the plain version, so the
limit is not the feature engineering — shared vocabulary in a 320-character
abstract simply does not separate these fields. That closes off the lexical
direction rather than inviting more of it.

**Not knowing how many folders to make costs more than any tuning.** Sweeping
the stopping threshold on the coherent 45 papers moves ARI from 0.104 (13
folders) down to 0.000 (45 folders) as the threshold rises, monotonically. No
setting rescues it, which is why the pipeline's model-chosen taxonomy has real
value: choosing *how many* groups exist is the hard part.

**v-measure moves opposite to ARI across that sweep** — rising from 0.31 to
0.46 as grouping gets worse and more split. Any quality claim resting on
v-measure, purity, or homogeneity alone is measuring fragmentation.

**Preprocessing is not the bottleneck.** An earlier review claimed
`preprocess_for_llm` discards the signal clustering needs. On the full 4-label
set the reduced terms scored *better* than the raw abstract (0.204 vs 0.110),
and on the 3-label subset worse (0.038 vs 0.085). At this corpus size the
effect is noise in both directions, so that claim is withdrawn: there is no
evidence the reduction is what limits grouping.

## Still open

Provider embeddings remain untested here — no API key and no local Ollama on
this machine. The latent-topic result above is the offline stand-in for them
and points the same way: semantic structure beats surface overlap against its
own baseline (0.050 to 0.158 on the 4-label set), but not by enough to separate
from noise on 45 papers.

Testing real embeddings is therefore worth doing *after* the balanced set
exists, not before — on this corpus the result would not be interpretable. It
is implemented and gated on a key:

```bash
SYP_API_KEY=... cargo test -p syp-library curated_set_embedding_bench -- --ignored --nocapture
```

That prints the term strategies and the embedding strategy over the same
papers, so the comparison is direct.

The corpus limits all of this, and the stability check puts a number on it:
abstracts are truncated to ~320 characters, 45–60 papers is small enough that
folds swing ARI by ±0.1, and one of four categories is incoherent by
construction. Building the balanced `clustering-eval` set is therefore a
prerequisite for ranking approaches, not a refinement of it — no amount of
further method invention can be evaluated on this data.
