# Embedding Comparison

This document compares two dry-run taxonomy/placement passes over the curated `scijudgebench-diverse` test set.

- Embedding-guided run: `run-1262478-1774397271681`
- LLM-only run: `run-1263307-1774397508098`
- Reference DB: `~/.local/share/sortyourpapers/paperdb.duckdb`
- Reference evidence present in embedding-guided run: `yes`
- Reference evidence present in llm-only run: `no`
- Final taxonomy path count: embedding-guided `21`, llm-only `36`
- Placement changes between runs: `57` of `59` papers

## Final Taxonomy Summary

- Shared final paths: `6`
- Embedding-guided only paths: `15`
- LLM-only only paths: `30`

### Shared Paths

- `Biology`
- `Computer Science`
- `Engineering`
- `Mathematics`
- `Mathematics / Applied Mathematics`
- `Physics`

### Embedding-Guided Only Paths

- `Biology / Bioengineering and Biophysics`
- `Biology / Molecular Biology and Endocrinology`
- `Computer Science / Artificial Intelligence and Machine Learning`
- `Computer Science / Computer Vision and Robotics`
- `Computer Science / Natural Language Processing`
- `Computer Science / Software and Information Science`
- `Engineering / Aerospace Engineering`
- `Engineering / Biomedical Engineering`
- `Engineering / Materials Science`
- `Mathematics / Algebra and Geometry`
- `Mathematics / Probability and Statistics`
- `Physics / Applied and Computational Physics`
- `Physics / Astronomy and Cosmology`
- `Physics / Condensed Matter Physics`
- `Physics / Theoretical and Particle Physics`

### LLM-Only Only Paths

- `Astronomy`
- `Astronomy / Cosmology`
- `Biology / Biophysics`
- `Biology / Genetics`
- `Biology / Zoology`
- `Computer Science / Artificial Intelligence`
- `Computer Science / Data Science`
- `Computer Science / Software Engineering`
- `Computer Science / Theoretical Computer Science`
- `Computer Vision`
- `Computer Vision / Visual Odometry`
- `Engineering / Biomedical Signal Processing`
- `Engineering / Computational Science`
- `Engineering / Control Systems`
- `Engineering / Photonics`
- `General`
- `General / Academic Infrastructure`
- `Materials Science`
- `Materials Science / Nanostructures`
- `Mathematics / Algebra`
- `Mathematics / Analysis`
- `Mathematics / Geometry`
- `Physics / Applied Physics`
- `Physics / Condensed Matter`
- `Physics / Particle Physics`
- `Physics / Theoretical Physics`
- `Statistics`
- `Statistics / Bayesian Statistics`
- `Statistics / Geometric Statistics`
- `Statistics / Multivariate Data Analysis`

## Changed Placements

| LLM_only_category | embedding_informed_category | arxiv_id | arxiv title | paper_url |
| --- | --- | --- | --- | --- |
| Astronomy/Cosmology | Physics/Astronomy and Cosmology | 1807.06209 | Planck 2018 results. VI. Cosmological parameters | https://arxiv.org/abs/1807.06209 |
| Astronomy/Cosmology | Physics/Astronomy and Cosmology | astro-ph/9805201 | Observational Evidence from Supernovae for an Accelerating Universe and a Cosmological Constant | https://arxiv.org/abs/astro-ph/9805201 |
| Astronomy/Cosmology | Physics/Theoretical and Particle Physics | gr-qc/0004072 | Equivalent frames in Brans-Dicke theory | https://arxiv.org/abs/gr-qc/0004072 |
| Biology/Biophysics | Biology/Bioengineering and Biophysics | q-bio/0703042 | An allosteric model for circadian KaiC phosphorylation - Supporting Information | https://arxiv.org/abs/q-bio/0703042 |
| Biology/Genetics | Biology/Molecular Biology and Endocrinology | q-bio/0703056 | Quantitative Characterization of Combinatorial Transcriptional Control of the Lactose Operon of E. coli | https://arxiv.org/abs/q-bio/0703056 |
| Biology/Zoology | Biology/Molecular Biology and Endocrinology | q-bio/0703064 | Triiodothyronine suppresses humoral immunity but not T-cell-mediated immune response in incubating female eiders (Somateria mollissima) | https://arxiv.org/abs/q-bio/0703064 |
| Computer Science | Computer Science/Natural Language Processing | cs/9908001 | Detecting Sub-Topic Correspondence through Bipartite Term Clustering | https://arxiv.org/abs/cs/9908001 |
| Computer Science/Artificial Intelligence | Computer Science/Artificial Intelligence and Machine Learning | 1312.6114 | Auto-Encoding Variational Bayes | https://arxiv.org/abs/1312.6114 |
| Computer Science/Artificial Intelligence | Computer Science/Artificial Intelligence and Machine Learning | 1412.6980 | Adam: A Method for Stochastic Optimization | https://arxiv.org/abs/1412.6980 |
| Computer Science/Artificial Intelligence | Computer Science/Artificial Intelligence and Machine Learning | 1503.02531 | Distilling the Knowledge in a Neural Network | https://arxiv.org/abs/1503.02531 |
| Computer Science/Artificial Intelligence | Computer Science/Artificial Intelligence and Machine Learning | 1607.06450 | Layer Normalization | https://arxiv.org/abs/1607.06450 |
| Computer Science/Artificial Intelligence | Computer Science/Artificial Intelligence and Machine Learning | 1710.10903 | Graph Attention Networks | https://arxiv.org/abs/1710.10903 |
| Computer Science/Artificial Intelligence | Computer Science/Natural Language Processing | 1810.04805 | BERT: Pre-training of Deep Bidirectional Transformers for Language Understanding | https://arxiv.org/abs/1810.04805 |
| Computer Science/Artificial Intelligence | Computer Science/Artificial Intelligence and Machine Learning | 1911.00972 | Privacy for Free: Communication-Efficient Learning with Differential Privacy Using Sketches | https://arxiv.org/abs/1911.00972 |
| Computer Science/Artificial Intelligence | Computer Science/Natural Language Processing | cs/9811022 | Expoiting Syntactic Structure for Language Modeling | https://arxiv.org/abs/cs/9811022 |
| Computer Science/Artificial Intelligence | Computer Science/Artificial Intelligence and Machine Learning | cs/9905004 | Using Collective Intelligence to Route Internet Traffic | https://arxiv.org/abs/cs/9905004 |
| Computer Science/Data Science | Computer Science/Artificial Intelligence and Machine Learning | 1802.03426 | UMAP: Uniform Manifold Approximation and Projection for Dimension Reduction | https://arxiv.org/abs/1802.03426 |
| Computer Science/Software Engineering | Engineering/Materials Science | 0906.2569 | Quantum ESPRESSO: a modular and open-source software project for quantum simulations of materials | https://arxiv.org/abs/0906.2569 |
| Computer Science/Software Engineering | Computer Science/Software and Information Science | 2404.01240 | AURORA: Navigating UI Tarpits via Automated Neural Screen Understanding | https://arxiv.org/abs/2404.01240 |
| Computer Science/Theoretical Computer Science | Computer Science/Software and Information Science | cs/9911009 | Two-way finite automata with quantum and classical states | https://arxiv.org/abs/cs/9911009 |
| Computer Vision | Computer Science/Computer Vision and Robotics | 1409.1556 | Very Deep Convolutional Networks for Large-Scale Image Recognition | https://arxiv.org/abs/1409.1556 |
| Computer Vision | Computer Science/Computer Vision and Robotics | 1506.06825 | DeepStereo: Learning to Predict New Views from the World's Imagery | https://arxiv.org/abs/1506.06825 |
| Computer Vision | Computer Science/Computer Vision and Robotics | 1512.03385 | Deep Residual Learning for Image Recognition | https://arxiv.org/abs/1512.03385 |
| Computer Vision | Computer Science/Computer Vision and Robotics | 2010.11929 | An Image is Worth 16x16 Words: Transformers for Image Recognition at Scale | https://arxiv.org/abs/2010.11929 |
| Computer Vision/Visual Odometry | Computer Science/Computer Vision and Robotics | 1908.08814 | Multi-Spectral Visual Odometry without Explicit Stereo Matching | https://arxiv.org/abs/1908.08814 |
| Engineering/Biomedical Signal Processing | Engineering/Biomedical Engineering | 1911.11610 | Improving EEG based Continuous Speech Recognition | https://arxiv.org/abs/1911.11610 |
| Engineering/Computational Science | Computer Science/Artificial Intelligence and Machine Learning | 1509.03580 | Discovering governing equations from data: Sparse identification of nonlinear dynamical systems | https://arxiv.org/abs/1509.03580 |
| Engineering/Computational Science | Engineering/Materials Science | 2407.19326 | Accounting for plasticity: An extension of inelastic Constitutive Artificial Neural Networks | https://arxiv.org/abs/2407.19326 |
| Engineering/Control Systems | Engineering/Aerospace Engineering | 2107.04094 | Robust Control Barrier Functions under High Relative Degree and Input Constraints for Satellite Trajectories | https://arxiv.org/abs/2107.04094 |
| Engineering/Photonics | Physics/Applied and Computational Physics | 1811.03455 | High fidelity single-pixel imaging | https://arxiv.org/abs/1811.03455 |
| General/Academic Infrastructure | Computer Science/Software and Information Science | cs/9909003 | Iterative Deepening Branch and Bound | https://arxiv.org/abs/cs/9909003 |
| Materials Science/Nanostructures | Physics/Condensed Matter Physics | cond-mat/0410550 | Electric Field Effect in Atomically Thin Carbon Films | https://arxiv.org/abs/cond-mat/0410550 |
| Mathematics/Algebra | Mathematics/Algebra and Geometry | 1007.2372 | L-R-smash products and L-R-twisted tensor products of algebras | https://arxiv.org/abs/1007.2372 |
| Mathematics/Algebra | Mathematics/Algebra and Geometry | 1912.03019 | Unramified Heisenberg group extensions of number fields | https://arxiv.org/abs/1912.03019 |
| Mathematics/Algebra | Mathematics/Algebra and Geometry | q-alg/9712027 | Coherence Constraints for Operads, Categories and Algebras | https://arxiv.org/abs/q-alg/9712027 |
| Mathematics/Algebra | Mathematics/Algebra and Geometry | q-alg/9712039 | Genus-zero modular functors and intertwining operator algebras | https://arxiv.org/abs/q-alg/9712039 |
| Mathematics/Algebra | Mathematics/Algebra and Geometry | q-alg/9712053 | Skew divided difference operators and Schubert polynomials | https://arxiv.org/abs/q-alg/9712053 |
| Mathematics/Analysis | Mathematics/Algebra and Geometry | 1011.1669 | A "missing" family of classical orthogonal polynomials | https://arxiv.org/abs/1011.1669 |
| Mathematics/Analysis | Mathematics/Applied Mathematics | solv-int/9912011 | Liouville equation under perturbation | https://arxiv.org/abs/solv-int/9912011 |
| Mathematics/Applied Mathematics | Engineering | 1408.4408 | A Data-Driven Approximation of the Koopman Operator: Extending Dynamic Mode Decomposition | https://arxiv.org/abs/1408.4408 |
| Mathematics/Geometry | Mathematics/Algebra and Geometry | math/0211159 | The entropy formula for the Ricci flow and its geometric applications | https://arxiv.org/abs/math/0211159 |
| Mathematics/Geometry | Mathematics/Algebra and Geometry | q-alg/9709040 | Deformation quantization of Poisson manifolds, I | https://arxiv.org/abs/q-alg/9709040 |
| Physics/Applied Physics | Physics/Applied and Computational Physics | 2008.04464 | Frequency Selective Wave Beaming in Nonreciprocal Acoustic Phased Arrays | https://arxiv.org/abs/2008.04464 |
| Physics/Applied Physics | Biology/Bioengineering and Biophysics | q-bio/0703055 | The microtubule transistor | https://arxiv.org/abs/q-bio/0703055 |
| Physics/Condensed Matter | Physics/Condensed Matter Physics | 0709.1163 | The electronic properties of graphene | https://arxiv.org/abs/0709.1163 |
| Physics/Condensed Matter | Physics/Condensed Matter Physics | supr-con/9507005 | Constants of motion in the dynamics of a 2N-junction SQUID | https://arxiv.org/abs/supr-con/9507005 |
| Physics/Condensed Matter | Physics/Condensed Matter Physics | supr-con/9511001 | Synchronization in one-dimensional array of Josephson coupled thin layers | https://arxiv.org/abs/supr-con/9511001 |
| Physics/Condensed Matter | Physics/Condensed Matter Physics | supr-con/9604002 | Low temperature thermal conductivity of Zn-doped YBCO: evidence for impurity-induced electronic bound states | https://arxiv.org/abs/supr-con/9604002 |
| Physics/Particle Physics | Physics/Theoretical and Particle Physics | 2110.15839 | Transverse momentum distributions in low-mass Drell-Yan lepton pair production at NNLO QCD | https://arxiv.org/abs/2110.15839 |
| Physics/Particle Physics | Physics/Theoretical and Particle Physics | 2212.04393 | Automated NLO Electroweak Corrections to Processes at Hadron and Lepton Colliders | https://arxiv.org/abs/2212.04393 |
| Physics/Theoretical Physics | Physics/Theoretical and Particle Physics | q-alg/9703017 | Traces of creation-annihilation operators and Fredholm's formulas | https://arxiv.org/abs/q-alg/9703017 |
| Physics/Theoretical Physics | Physics/Theoretical and Particle Physics | q-alg/9712056 | Solutions of the qKZB Equation in Tensor Products of finite dimensional modules over the elliptic quantum group $E_{\tau,\eta}sl_2$ | https://arxiv.org/abs/q-alg/9712056 |
| Physics/Theoretical Physics | Physics/Theoretical and Particle Physics | solv-int/9912002 | Quantum Lax scheme for Ruijsenaars models | https://arxiv.org/abs/solv-int/9912002 |
| Statistics | Mathematics/Probability and Statistics | 1405.0676 | Concentration via chaining method and its applications | https://arxiv.org/abs/1405.0676 |
| Statistics/Bayesian Statistics | Mathematics/Probability and Statistics | 1511.05650 | Tree-Guided MCMC Inference for Normalized Random Measure Mixture Models | https://arxiv.org/abs/1511.05650 |
| Statistics/Geometric Statistics | Mathematics/Probability and Statistics | 2306.09740 | Spatial depth for data in metric spaces | https://arxiv.org/abs/2306.09740 |
| Statistics/Multivariate Data Analysis | Mathematics/Probability and Statistics | 2006.14411 | Categorical Exploratory Data Analysis: From Multiclass Classification and Response Manifold Analytics perspectives of baseball pitching dynamics | https://arxiv.org/abs/2006.14411 |
