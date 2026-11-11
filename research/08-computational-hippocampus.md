# Research 08: Computational hippocampus, mapped to engineering mechanisms

> Research thread report, checked 2026-11-10. Produced by a research agent from public sources and saved verbatim. Evidence labels: **[V]** verified in a primary source, **[C]** project or vendor claim, **[I]** independent evidence, **[Inf]** inference, **[?]** unknown. Synthesis and decisions live in [`docs/`](../docs/).

## Executive summary

- **The current design holds up.** hivemind already does three things the research supports: it records episodes quickly in an append-only log, consolidates them slowly and optionally, and retrieves through compact cards that point to evidence. These follow complementary learning systems theory and hippocampal indexing theory, both broadly accepted.
- **Most useful new idea: separate how memories are found from what they store.** Gershman, Fiete and Irie (2025) argue that retrieval keys should be optimised to tell memories apart, while stored content is optimised for accuracy. For hivemind, cards are the keys and evidence is the content, so keys can be enriched with entities, error signatures and applicability conditions without touching immutable evidence. LongMemEval reports gains from exactly this kind of key expansion.
- **Consolidate selectively.** A 2023 model (Sun et al.) shows that copying every memory into long-term knowledge causes overfitting; only regularities that generalise should be consolidated. A 2026 agent paper (Hippo) reports that piling up experiences without consolidation first helps, then drops below having no memory at all.
- **Cut sessions into episodes at event boundaries.** Hippocampal activity peaks at boundaries, and the just-finished episode is replayed at that moment. For coding sessions, simple signals work: task or branch changes, commits, and tests flipping between failing and passing.
- **Use surprise to weight memories, not to filter them.** Prediction errors strengthen memory, but things that fit existing knowledge are also remembered well. Dropping unsurprising writes would lose dull but critical facts such as build commands.
- **Two concrete rules from pattern separation and completion:**
  - Keep near-duplicate memories that apply in different contexts as separate claims, each stating what distinguishes it.
  - Optionally fill in partial cues with graph search over entities, as HippoRAG does with personalised PageRank. An independent benchmark found this helps complex multi-hop questions, while plain retrieval is better for simple facts.
- **The reliable lesson from reconsolidation is computational.** Human reconsolidation results are contested. The latent-cause model gives a usable rule: update an old memory only when the new observation shares its context; otherwise create a new one. In hivemind terms, supersede only when applicability matches.
- **Replay research suggests how to schedule consolidation:** priority equals expected improvement times the chance the memory will be needed again. Offline pre-computation ("sleep-time compute") paid off when future queries were predictable, but on its software-engineering case study it helped only at low test-time budgets.
- **Forgetting should lower a memory's rank, not delete it.** On one benchmark's conflict-resolution questions, a simple "newest statement wins" rule solved about 80% of items, while every memory system scored at most 28% on the multi-hop subset. Explicit supersession matters more than biologically inspired decay.
- **Evidence for hippocampus-inspired AI systems is thin.** Most gains are reported only by their authors. On real coding tasks, 11 of 12 combinations of memory system and agent failed to beat running with no memory.

**Recommendations**

- **Build now (v0):**
  - cards as keys plus an entity index;
  - episodes cut at boundaries;
  - surprise used as a weight;
  - showing what happened next in the same episode;
  - feedback on retrieved memories, with a "contested" state;
  - keeping near-duplicates separate;
  - rank-only decay;
  - explicit provenance;
  - declining to return memories when confidence is low.
- **Test first (v1):** graph search for partial cues, a consolidation scheduler, consolidating only what generalises, fast-tracking memories that fit known patterns, "what usually comes next" statistics, linking memories written close in time, and procedures with fill-in slots.
- **Reject:** neural associative stores, memory held in model weights at test time, generative replay, literal sleep cycles, regenerating evidence, and filtering writes by surprise alone.

---

## Research 08: Computational hippocampus → engineering mechanisms for hivemind

> Research thread report, checked 2026-11-10. Public sources only. **Method:** every DOI was resolved through the Crossref API, abstracts were read through PubMed or Europe PMC and the arXiv API, and targeted full-text checks were done for Mattar & Daw 2018 (PMC6203620), GraphRAG-Bench, MemoryAgentBench, EM-LLM, Titans, Sleep-time Compute, Hippo (2610.24453), and Rainbow (ar5iv). **Labels:** **[V]** verified in the primary source (abstract unless "full text" is stated); **[V-bib]** existence and bibliographic details verified, content not re-read here (any finding attached carries its own label); **[C]** authors' claim, not independently checked; **[I]** independent replication or broad consensus; **[Debated]** contested in the field; **[Inf]** our inference; **[?]** unknown. Numbers in brackets refer to §7 Sources.

## 1. Summary

1. **hivemind's current "hippocampal" commitments are well-founded.** Fast append-only episodes, slow optional consolidation, and an index of cards that point to evidence match complementary learning systems (CLS) [1,2] and indexing theory [6,7]. Both are broadly accepted frameworks [I]. Newer theory sharpens them rather than replacing them.
2. **Key–value memory is the most useful single new idea** [8]. A key–value memory separates retrieval *keys*, optimised for discriminability, from stored *values*, optimised for fidelity.
   - In hivemind, cards are keys and evidence is values. Keys can be enriched (entities, error signatures, applicability) without touching immutable values.
   - In LLM memory, LongMemEval reports gains from key expansion and session decomposition [V, 174].
3. **Consolidate selectively.**
   - Generalisation-optimised CLS ("Go-CLS"): unregulated transfer from hippocampus to neocortex overfits, so memories should consolidate only when that helps generalisation [V, 3].
   - A 2026 agent-memory paper finds that accumulating experience without consolidation follows an inverted U and ends below no-memory [C, 182].
4. **Event boundaries are the natural unit of an episode.**
   - Hippocampal responses peak at boundaries [V, 72, 73].
   - Boundaries trigger reinstatement of the just-finished episode [V, 74].
   - In a model, encoding at the end of an event beats encoding mid-event [V, 78].
   - In LLM memory, segment-level memory helps [C, 146, 176].
5. **Prediction error (PE) and novelty should *weight* encoding, not *gate* it.** They enhance encoding [V, 27, 32, 34], but schema-congruent information is also remembered better [V, 52]. The brain modulates strength; it does not discard the unsurprising.
6. **Pattern separation and completion become two policies:**
   - keep near-duplicates that differ in context distinct, with discriminators (the hippocampus actively pushes overlapping memories apart [V, 22, 23]);
   - complete partial cues by graph spreading over entities (HippoRAG). Independent evidence: graph retrieval helps multi-hop and complex questions and loses on simple facts [I, 144].
7. **Reconsolidation's dependable lesson is computational.**
   - Human evidence is contested [V, 49, 50].
   - The latent-cause model [V, 48] gives the engineering rule: modify an old memory only when the new observation is inferred to share its cause (in hivemind, its applicability); otherwise create a new memory.
8. **Replay gives a consolidation scheduler:** priority = gain × need [V full text, 62].
   - Human data support it [V, 63]; a planning role is disputed [V, 64].
   - Sleep-time compute pays off when future queries are predictable. On a software-engineering task it helped only at low test-time budgets [C/V full text, 162].
9. **Temporal contiguity is among the most robust regularities in memory** [V, 82] and appears in transformer induction heads [V, 86]. Returning temporal neighbours on expansion is nearly free.
10. **Forgetting should reduce accessibility, not delete** [V, 110, 112], and should track the probability a memory will be needed [V, 116]. On benchmarks, explicit supersession dominates: a last-write rule answers about 80% of MemoryAgentBench conflict items [I, 191], where all evaluated systems scored at most 28% on the multi-hop setting [I, 190].
11. **Provenance is source monitoring done right.** Human source attributions are fallible inferences [V, 120], and consolidation adds schema distortions [V, 124]. Store sources explicitly and never regenerate evidence.
12. **Independent evidence for hippocampus-inspired AI memory is thin.** On real repository coding tasks, 11 of 12 memory-system/solver pairings failed to beat no memory [I, 192].
13. **Decision:**
    - **v0 (build):** keys and entity index, boundary episodes, salience weighting, temporal-neighbour expansion, retrieval feedback with a "contested" state, separation of near-duplicates, accessibility decay, provenance, and abstention.
    - **v1 (test):** personalised PageRank (PPR) completion, gain×need consolidation, Go-CLS selective consolidation, schema fast-track, successor links, co-temporal linking, slotted procedures.
    - **Reject:** neural associative stores, test-time weight memory, generative replay, sleep literalism, generative evidence reconstruction, pure surprise gating.

---

## 2. Part A — Hippocampal computation: theories, evidence and status

### A1. Complementary learning systems (CLS)
- **Claim.**
  - The hippocampus stores recent memories by fast synaptic change and reinstates them in neocortex.
  - Neocortex changes "a little on each reinstatement", learning structure gradually through interleaving.
  - This avoids disrupting existing structure [V, 1].
- **2016 update** [V, 2]:
  - replay allows "goal-dependent weighting of experience statistics";
  - recurrent activation of hippocampal traces can support some generalisation;
  - neocortical learning can be rapid for information consistent with known structure.
- **Status.** Broad consensus as an organising framework [I]. What transfers, and when, remains open.
- **Refinements:**
  - **Go-CLS (2023)** [V, 3]: unregulated neocortical transfer "can cause overfitting and harm generalization in an unpredictable world". Memories consolidate only when this aids generalisation, which explains partial transfer.
  - **CLS inside the hippocampus** [V, 4]: in a model, the monosynaptic entorhinal–CA1 pathway learns statistics while the trisynaptic pathway learns episodes.
  - **REMERGE** [V, 5]: generalisation can arise at *retrieval time* from recurrent similarity among stored episodes, without consolidation.
- **For hivemind:** keep the dual store. Test selective consolidation (C13) against on-demand aggregation over episodes (C17).

### A2. Hippocampal memory indexing theory → key–value memory
- **Claim.**
  - The hippocampus captures an index of the neocortical activity an episode produced.
  - A partial cue activates the index, which reinstates the full pattern.
  - In the 2007 update the authors conclude the theory "has aged very well" [V, 7]; original paper [V-bib, 6].
- **Status.** Consistent with engram reactivation work [V, 109] [I]. Contested only through the consolidation debate (A5).
- **2025 key–value reformulation** [V, 8]: keys (retrieval) and values (storage) are distinct representations, which lets a system "optimize simultaneously for fidelity in storage and discriminability in retrieval". This is a theoretical review; direct neural tests are pending [Inf].
- **Use in AI.** HippoRAG explicitly operationalises indexing theory [V, 142]. LongMemEval's "fact-augmented key expansion" and "session decomposition for value granularity" improve recall and QA [V, 174].
- **For hivemind:** C1. Cards are keys; episodes and artifacts are values.

### A3. Marr; pattern separation (dentate gyrus) and pattern completion (CA3)
- **Classic models** [V-bib, 9, 10, 11] [I as textbook model]:
  - the dentate gyrus does sparse expansion recoding, which separates similar inputs;
  - CA3 recurrent collaterals form an autoassociative attractor, which completes partial inputs.
- **Evidence:**
  - Rat dentate gyrus decorrelates activity under minimal changes to the environment. Larger changes recruit new CA3 populations [V, 13].
  - In humans, CA3/dentate gyrus shows a bias toward separation, while CA1 and cortex show a bias toward completion (fMRI) [V, 14].
  - Mice with CA3 NMDA receptors knocked out cannot recall from partial cues but are normal with full cues [V, 15].
  - Reviews describe convergent evidence with cross-species and measurement caveats [V, 12]. A 2023 framework argues separation is multistage and involves cognitive-control regions [V, 16]. Where it happens is **[Debated]**.
- **Learning-driven differentiation** (directly relevant to near-duplicates):
  - With learning, hippocampal patterns for similar events move apart, and lower overlap reduces later interference [V, 22].
  - Overlapping routes "repel" until they are *less* similar than non-overlapping ones [V, 23].
  - Nonmonotonic plasticity: moderate co-activation weakens and differentiates; strong activation strengthens and integrates [V, 24].
  - A 2025 model of behavioural-timescale synaptic plasticity (one-shot, binary synapses) yields content-addressable memory and reproduces the repulsion effect [V, 25]. Experimental basis [V-bib, 136].
- **Adult neurogenesis.**
  - Mice: more adult neurogenesis improves separation [V, 17] and increases forgetting [V, 115].
  - Humans:
    - "undetectable" in adults [V, 18];
    - persists through aging [V, 19];
    - "abundant", dropping in Alzheimer's [V, 20];
    - proliferating progenitors found in adults by single-nucleus RNA sequencing plus machine learning [V, 21].
  - Status: **[Debated]**.
- **Engineering cousin:** the fly olfactory circuit implements a variant of locality-sensitive hashing (LSH) that improved similarity search [V, 26]. Its specific strategies (expansion, sparse binary projection, winner-take-all) are described in the paper body [V-bib].
- **For hivemind:**
  - Separation and completion are *opposing policies* to tune per query (C5 vs C4).
  - Binary or sparse codes are an implementation choice for near-duplicate detection, not a quality mechanism.

### A4. CA1 comparator, novelty, and the hippocampal–VTA loop
- **Lisman & Grace** [V, 27]: the hippocampus detects information "not already stored". The novelty signal travels via subiculum, accumbens and ventral pallidum to the ventral tegmental area (VTA). Dopamine released in the hippocampus enhances long-term potentiation, so the loop "regulates the entry of information into long-term memory".
- **CA1 as match/mismatch detector** [V, 28]: CA1 tracked the *number* of changes in a probe, including behaviourally irrelevant ones, consistent with an automatic comparator. Associative novelty, as distinct from item novelty [V, 29].
- **Where the dopamine comes from is [Debated]:**
  - Locus coeruleus neurons mediate the novelty-driven memory boost independently of the VTA [V, 30].
  - An opinion piece proposes two systems, VTA and locus coeruleus, for different kinds of novelty, with VTA tied to novelty that shares commonalities with past experience and to semantic memory [V, 31].
- **Behavioural evidence:**
  - recognition was better for items with larger absolute reward PE, dissociated from learning rate [V, 32];
  - high reward PEs create event boundaries [V, 33];
  - PE manipulations improved one-shot declarative encoding in all five experiments [V, 34];
  - acetylcholine sets encoding versus retrieval modes [V-bib, 35] [I];
  - a title-level claim of rapid plasticity in CA1 during novel experiences [V-bib, 137].
- **Countervailing:** schema-congruent information is often remembered *better*, and novelty helps only sometimes [V, 52]. The relation is plausibly U-shaped [Inf].
- **Status:** novelty and PE enhance encoding [I]; the circuit details are [Debated]. The effect is **graded**, not all-or-none.
- **For hivemind:** C3 (weighting) and C2 (PE as a boundary signal).

### A5. Systems consolidation: standard vs multiple-trace vs transformation (and contextual binding)

| Theory | Core claim | Notes |
|---|---|---|
| Standard (SSCT) [V-bib, 36; V, 40] | Memories initially depend on the hippocampus, then become neocortex-dependent | Supported by temporally graded retrograde amnesia |
| Multiple trace theory [V-bib, 37] | Detailed episodic memory always needs the hippocampus | Each retrieval re-encodes |
| Trace transformation [V, 38; V-bib, 39] | Episodic, context-specific memories *transform* into semantic or gist versions; the detailed version stays hippocampal | "changes in neural representation … accompanied by changes in the nature of the memory" |
| Contextual binding [V, 42] | Forgetting is largely contextual interference; episodic memory stays hippocampus-dependent; sleep reduces interference | 2019 challenge to SSCT |

- **Engram data:**
  - Prefrontal engram cells form *at learning* and mature over time, while hippocampal engrams become silent (rodent fear memory) [V, 41].
  - Engrams become *more selective* with consolidation [V, 43].
  - Consolidation is partial (Go-CLS [V, 3]).
- **Status:** **[Debated]** whether detailed episodic retrieval ever becomes hippocampus-independent. Consensus that memories transform and gist becomes cortically supported [I].
- **For hivemind:** the transformation and contextual-binding view is the right mental model. Keep episodes, let derived claims coexist, and always resolve detail to evidence. Never delete episodes because they were consolidated.

### A6. Reconsolidation and prediction-error-driven updating
- **Nader et al. (2000)** [V, 44]: reactivated, consolidated fear memories became labile. Blocking protein synthesis after reactivation caused amnesia; without reactivation memory was intact.
- **Prediction error is necessary for destabilisation** in human fear memory [V, 45]. Incomplete reminders generate PE, which lets memories be "distorted, updated, and strengthened" [V, 46]. Memory strength and age are boundary conditions [V, 47].
- **Latent-cause model** [V, 48]: new memories form when inference says a *new* latent cause is generating observations; old memories are modified when old and new observations are inferred to share a cause.
- **Human evidence is [Debated].** Four direct replications (n=64) of a widely cited motor-sequence result failed. Three conceptual replications (n=48) found that retrieval *protected* memories relative to no retrieval [V, 49]. A critical review offers a framework for judging such studies [V, 50].
- **Retrieval strengthens:** the testing effect [V-bib, 118] [I]; retrieval as "a rapid consolidation event" [V, 119].
- **For hivemind:** C6. Implement the latent-cause rule, not "biological lability". Updates are new events, never in-place edits.

### A7. Schemas accelerate consolidation
- **Tse et al. (2007)** [V, 51]: once rats had a schema, new associations trained in one trial became persistent and rapidly hippocampus-independent.
- **SLIMM** [V, 52]: congruent information is usually better remembered and incongruent information sometimes is. Medial prefrontal cortex supports congruent learning; the medial temporal lobe supports novel learning.
- Schemas can **enhance or distort** memory [V, 53]. Neocortical learning is fast for information consistent with known structure [V, 2].
- **Status:** influential rodent evidence and supportive human imaging [I]; schema distortions are well documented [I].
- **For hivemind:** C12, while guarding against schema-induced distortion.

### A8. Replay (sharp-wave ripples, awake vs sleep, reverse, prioritised)
- **Phenomena:**
  - sleep reactivation [V-bib, 54];
  - awake reverse replay [V-bib, 55];
  - forward sequences toward remembered goals [V-bib, 56];
  - reverse replay modulated by reward change [V-bib, 57];
  - disrupting ripples impairs memory [V-bib, 58], and prolonging them improves it [V-bib, 59];
  - reviews [V-bib, 60, 61, 140].
  - Replay exists and plays a causal role in consolidation [I].
- **Prioritised memory access** (Mattar & Daw) [V full text, 62]:
  - the value of a memory backup is the product of **gain** (how much the policy at that state improves) and **need** (expected discounted future visits to that state);
  - gain dominates after surprises, giving reverse sequences; need dominates ahead of the agent, giving forward sequences.
- **Human support:**
  - after reward, backward replay of nonlocal experience at a 160 ms lag, stronger for "experiences of greater benefit for future behavior" [V, 63];
  - flexible subjects replay online and inflexible ones offline [V, 66];
  - replay reorganises experience [V-bib, 67].
- **Contrary:** replay content was *decoupled* from the next choice and enriched for previously rewarded and not-recently-visited places, suggesting storage rather than planning [V, 64]. The planning role is **[Debated]**.
- **Tagging:** awake ripples during reward consumption decoded the surrounding trial blocks; during later sleep, ripples replayed the blocks most reactivated awake [V, 65].
- **Compositional and generative replay (2023–2025):**
  - replay assembles elements into hypotheses [V, 97];
  - replay builds compositional memories [V, 96];
  - review [V-bib, 98].
- **For hivemind:** C7. Tag during sessions; consolidate offline by gain×need.

### A9. Event segmentation and boundaries
- **Event segmentation theory:** a boundary is perceived when transient prediction errors arise [V, 68].
- **Memory is organised by boundaries:**
  - associative memory is lower across boundaries [V, 69];
  - order memory is better within events than across them [V, 70, 71].
- **The hippocampus responds at boundaries:**
  - the response scales with boundary salience and is specific to boundaries (n=253 and n=15, film viewing) [V, 72];
  - nested event hierarchy, where high-level boundaries couple to hippocampal activity that predicts later reinstatement [V, 73].
- **Boundaries trigger rapid reinstatement** (about 200–800 ms) of the *just-encoded* episode, which predicts later memory [V, 74].
- **Single neurons** detect abstract boundaries. Boundaries improved recognition but *impaired order memory*, a content-versus-time trade-off [V, 75].
- **Models:**
  - Structured Event Memory: probabilistic boundary inference plus event schemata [V, 77];
  - review [V-bib, 76];
  - reward PEs create boundaries [V, 33];
  - memory-augmented networks perform better when they encode at event *end* and retrieve selectively under uncertainty, because this reduces the "risk of retrieving irrelevant memories" [V, 78].
- **Status:** boundary effects are robust [I]; the computational accounts are [Debated] in detail.
- **For hivemind:** C2 and C18.

### A10. Temporal context model, time cells, contiguity
- **Temporal context model and its successor, the context maintenance and retrieval model (CMR)** [V-bib, 79; V, 81]: a drifting internal context binds items. Temporal, semantic and source clustering emerge.
- **Contiguity:**
  - [V-bib, 80];
  - a 34-finding benchmark shows contiguity across recognition, paired associates and autobiographical recall, over "minutes, days, weeks, and years" [V, 82];
  - review [V-bib, 139].
- **Time cells** [V-bib, 83, 84]. Human hippocampal and entorhinal neurons learn the *graph* of a sequence, reflect upcoming-stimulus probabilities, and show time-compressed replay [V, 85].
- **In transformers:** induction heads behave like CMR; CMR-like heads emerge in intermediate and late layers, and ablating them hurts in-context learning [V, 86].
- **Status:** contiguity is robust [I]; mechanism [Debated] (context drift vs. alternatives).
- **For hivemind:** C9.

### A11. Successor representation / predictive map
- **Successor representation** [V-bib, 87]. Place cells look like a predictive representation, with grid cells as a low-dimensional basis for it [V, 88]. Human behavioural and neural evidence [V-bib, 89, 90].
- **Status:** influential; competes with structure-learning accounts (A12) [Debated].
- **For hivemind:** C10 ("after X, agents usually do Y").

### A12. Tolman-Eichenbaum Machine; transformers and the hippocampal formation
- **Tolman-Eichenbaum Machine (TEM)** [V, 91]: medial entorhinal cells form a basis for *structure*, and hippocampal cells bind structure to sensory specifics. TEM predicts structural transfer across remapping, confirmed in recordings.
- **Transformers:** with recurrent position encodings they reproduce place and grid cells and are closely related to TEM [V, 92].
- **Related models and 2023–2025 work:**
  - clone-structured graphs [V-bib, 93, 94];
  - prefrontal "activity slots" unified with hippocampal maps [V, 95];
  - composition plus replay builds new behaviour with no new learning [V, 96].
- **For hivemind:** C19. Factor procedures (structure) from bindings (repo, version, OS).

### A13. Concept cells; cognitive maps for abstract spaces
- **Concept cells:** invariant single-neuron responses [V-bib, 99]. They are "sparse, explicit and abstract", proposed as building blocks for associations [V, 100]. Medial temporal lobe neurons expand their selectivity to a new associate within a few trials [V, 101].
- **Cognitive maps:** a framework for map-like organisation of non-spatial knowledge [V, 102]; a grid-like code during navigation of conceptual 2-D space [V, 103].
- **Status:** concept cells are robust in human recordings [I]; how to interpret them is [Debated].
- **For hivemind:** C8 (entity index, fast association).

### A14. Engram allocation and memory linking
- **Allocation by excitability** [V-bib, 104].
- **Linking in time:**
  - CA1 ensembles overlap more for contexts learned within a day than a week apart. Fear transfers between linked contexts; the effect is lost in aged mice and rescued by raising excitability [V, 105].
  - In the amygdala, events close in time are co-allocated and distal events are separated [V, 106].
  - The overlapping ensemble is needed to *link* memories but not to recall each one [V, 107].
  - **2025:** a strong aversive event drives *offline co-reactivation* that links it to a neutral memory from two days earlier. Linking is retrospective, not prospective, and co-reactivation occurs more during wake than sleep [V, 108].
  - Review [V, 109]. Engram selectivity grows with consolidation [V, 43].
- **Status:** robust in rodents, mostly fear paradigms [I]; translation to humans is indirect [Inf].
- **For hivemind:** C11.

### A15. Adaptive forgetting
- **Transience is functional:** it improves flexibility and prevents overfitting; "the goal of memory is … to optimize decision-making" [V, 110].
- **Mechanisms:**
  - active forgetting through neurogenesis, interference and intrinsic dopamine-driven pathways [V, 111];
  - forgetting as reversible loss of engram *accessibility*, sensitive to mismatches between expectation and environment [V, 112];
  - prefrontal control [V, 113];
  - retrieval-induced forgetting [V-bib, 114];
  - neurogenesis-driven forgetting [V, 115].
- **Rational analysis:** the probability a memory is needed follows frequency, recency and spacing in the environment, and memory matches those regularities [V, 116]. Formalised as ACT-R base-level activation [V-bib, 117].
- **Status:** adaptive forgetting is broadly accepted [I]; mechanisms are diverse.
- **For hivemind:** C14 (decay changes ranking, not existence).

### A16. Source monitoring and reconstructive memory
- **Source monitoring** [V, 120]: attributions are *inferences* from the qualities of an experience, judged by flexible criteria and "subject to error and disruption".
- **Constructive memory:** misattribution and suggestibility [V-bib, 121], constructive memory and future thinking [V-bib, 122], and lure words in false-memory lists [V-bib, 123] [I].
- Boundary-related neural states are reinstated in *false* memories too [V, 75]. Schema-based distortions *increase with consolidation* [V, 124].
- **For hivemind:** C15. Do not emulate this. Provenance makes source explicit, and consolidated outputs must be distrusted relative to evidence.

### A17. Generative models of memory construction; memory as compression
- **Spens & Burgess (2024)** [V, 124]:
  - hippocampal replay trains generative models (variational autoencoders) that reconstruct experiences;
  - predictable conceptual elements are reconstructed from cortex, while the hippocampus is used for "new and unusual information";
  - this explains imagination and schema distortions.
- **Semantic compression** with a generative model explains gist distortions and expertise effects [V, 125].
- **Multi-timescale synaptic cascades** move memories from fast to slow variables, with capacity scaling near-linearly in synapse count [V, 126; V-bib, 127, 128].
- **For hivemind:** C16. Store *residuals*, meaning what cannot be derived. Never regenerate evidence.

### A18. Associative memory models
- **Classic:** Hopfield networks [V-bib, 129]; Kanerva's Sparse Distributed Memory (SDM), MIT Press 1988 [V-bib, 130].
- **Modern:**
  - dense associative memory stores many more patterns than neurons [V, 131], with exponential capacity [V-bib, 132];
  - modern Hopfield networks: the update rule *is* attention, with exponential capacity, one-step retrieval, and "metastable states averaging over a subset of patterns" [V, 133];
  - attention approximates SDM under conditions met by GPT-2 [V, 134].
- **2025 Vector-HaSH** [V, 135]: factoring content storage from a grid-cell scaffold avoids the "memory cliff" and gives a *graceful trade-off between number of items and recall detail*.
- **For hivemind:** use these as design lessons, not storage engines (R1):
  - sharpness controls separation versus blending, and blended "metastable" retrievals are a model of gist confabulation;
  - graceful degradation corresponds to progressive disclosure.

### A19. Notable 2023–2026 advances

| Year | Work | Point for hivemind |
|---|---|---|
| 2023 | Go-CLS [V, 3] | Consolidate only what generalises |
| 2023 | Generative replay for compositional inference [V, 97]; replay & composition review [V-bib, 98]; extra-hippocampal pattern separation [V, 16] | Replay as hypothesis construction; separation needs control |
| 2024 | Generative consolidation [V, 124]; ripple tagging [V, 65]; human temporal-structure neurons [V, 85]; selective engrams [V, 43]; induction heads ≈ CMR [V, 86]; sequential predictive learning preprint [V-bib, 138] | Tag-then-consolidate; contiguity; predictive structure |
| 2025 | Key–value memory [V, 8]; Vector-HaSH [V, 135]; offline linking across days [V, 108]; structured slots [V, 95]; composition + replay [V, 96]; behavioural-timescale-plasticity content-addressable memory [V, 25]; adult progenitors [V, 21] | Key/value split; graceful degradation; retrospective linking |

---

## 3. Part B — AI/ML systems that operationalise these ideas

| System | Idea borrowed | Mechanism | Reported result (baseline, benchmark) | Cost | Evidence |
|---|---|---|---|---|---|
| **HippoRAG** (NeurIPS 2024) [142] | Indexing; CA3 completion | LLM open information extraction (OpenIE) builds a knowledge graph; personalised PageRank (PPR) from query entities | Up to 20% better than state of the art on multi-hop QA; single-step retrieval comparable to or better than the iterative IRCoT at 10–30× lower cost and 6–13× faster | LLM extraction at indexing time | [C]; independent: graph RAG wins on Level 2–3 questions (HippoRAG evidence recall 87.9–90.9%), vanilla RAG wins on simple facts (83.2% evidence recall) [I, 144]; see also [145] |
| **HippoRAG 2** (ICML 2025) [143] | Same, plus "recognition memory" | Passage and phrase nodes, query-to-triple matching, LLM filter of seed triples | +7% on associative tasks vs NV-Embed-v2; "no deterioration" on factual and sense-making tasks | Denser graphs (≈2.3k edges per 10k tokens) [I, 144]; compact prompts ≈10³ tokens vs 4×10⁴ for MS-GraphRAG global [I, 144] | [C] + partial [I]; slow memory construction at small chunk sizes [I, 190] |
| **EM-LLM** (ICLR 2025) [146] | Event segmentation; temporal context | Bayesian-surprise threshold plus graph-theoretic boundary refinement; k-NN retrieval plus a *contiguity buffer* of neighbouring events | Beats InfLLM on LongBench and ∞-Bench; beats RAG in most tasks; beats full context in most; retrieval over 10M tokens; boundaries correlate with human event boundaries | Works inside open-weight KV caches | [C]; no independent replication found [?] |
| **Larimar** (ICML 2024) [147] | Fast episodic memory (Kanerva Machine [148]) | Distributed memory for one-shot fact edits | Editing accuracy comparable to baselines with 8–10× speed-ups; selective forgetting | Model integration | [C]; not applicable to black-box agents |
| **Titans** [149] | Surprise-gated memorisation | Neural memory updated at test time; surprise = gradient of an associative loss, with momentum and decay | Beats Transformers and linear RNNs; >2M-token needle-in-a-haystack | Training required | [C]; reimplementation: not always better (chunking), but the memory module consistently helps vs attention-only [I, 150] |
| **Prioritized Experience Replay (PER)** (ICLR 2016) [153] | Prioritised replay | Sample transitions by TD error | Beats uniform-replay DQN on 41/49 Atari games | Priority queue | [C]; in Rainbow's ablation, prioritised replay and multi-step learning were "the two most crucial components" [V full text, 154] (same lab); widely adopted [I]. Background: [151, 152] |
| **Generative / brain-inspired replay** [155, 156] | Hippocampus as generator | Replay generated or internal representations to prevent catastrophic forgetting | Generative replay works on toy problems but scaling is "challenging"; brain-inspired variant reports state of the art on class-incremental CIFAR-100 without storing data [V, 156] | Training a generator | [C]; continual-learning context [157, 158, 159] |
| **Episodic control** [160, 161] | Hippocampal one-shot control | Nonparametric lookup of past returns | Faster early learning than DQN | Table memory | [C]; review [V-bib, 141] |
| **Sleep-time compute** [162] | Offline "sleep" processing | Pre-compute inferences about the context before queries arrive | ≈5× less test-time compute at equal accuracy; up to +13%/+18% accuracy; 2.5× cheaper per query when amortised; efficacy correlates with query predictability; **SWE-Features:** ≈1.5× fewer test-time tokens at low budgets, but test-time-only was *better* at high budgets | Background tokens | [C]; SWE nuance [V full text] |
| **Generative Agents** [163] | Recency, importance, relevance; reflection | Memory stream, reflection, planning | Ablation: each component contributes to human-rated believability | High | [C] |
| **Reflexion** [164]; **ExpeL** [165]; **Agent Workflow Memory (AWM)** [166]; **ReasoningBank** [167]; **ACE** [168]; **Dynamic Cheatsheet** [169] | Episodic → semantic/procedural consolidation; learning from failure | Reflections; insight extraction; induced workflows; strategies from successes and failures; incremental "playbooks" | HumanEval 91% vs 80%; AWM +24.6%/+51.1% relative (Mind2Web/WebArena); ACE +10.6% on agents and avoids "context collapse"; DC Game of 24: 10%→99% | LLM | [C] |
| **A-MEM** (NeurIPS 2025) [170] | Reconsolidation-like "memory evolution" | New notes update old notes' attributes | Beats baselines on six models | LLM per write | [C]; failed to beat no-memory on coding [I, 192] |
| **Mem0** [171] / **Zep** [172] | CLS-ish; bitemporal graph | Extraction and update; temporal knowledge graph | Mem0: +26% relative LLM-judge score over OpenAI memory, −91% p95 latency vs full context. Zep: DMR 94.8 vs 93.4; up to +18.5% on LongMemEval | LLM per write | [C]; the two vendors contest each other's numbers [Debated] |
| **MemoryBank** [173] | Ebbinghaus decay | Time and significance decay | Qualitative chatbot evaluation | Low | [C] |
| **SeCom** [176]; **LightMem** (ICLR 2026) [177]; **Nemori** [178] | Event segmentation; offline consolidation; prediction-error distillation | Topic segments; topic-grouped short-term memory with offline update; narrative episodes plus PE-based semantic distillation | SeCom beats turn-, session- and summary-level memory on LoCoMo; LightMem up to +7.7%/+29.3% QA with 38×/20.9× fewer tokens | Low–Med | [C] |
| **HippoMM** [179]; **HEMA** [180]; **Hippocampus** [181]; BMAM [183]; HiMeS [184]; AHN [185] | Various "hippocampal" labels | Segmentation plus temporal-window expansion; age-weighted pruning; binary signatures plus wavelet index; timeline episodes; learned compression | HippoMM 78.2% on its own benchmark; HEMA pruning −34% latency; Hippocampus up to 31× lower latency and 14× fewer tokens at matched accuracy | Mixed | [C]; mostly single-group, own benchmarks |
| **Hippo** (Oct 2026) [182] | Selective encoding plus replay | "Surprise-driven episodic buffer" (failures, or successes for the wrong reasons); compare attempts; merge with **explicit applicability conditions** | Gains on SWE-bench Verified, WebArena and Mind2Web with fewer retained experiences; the accumulation baseline follows an inverted U and falls *below no-memory* | LLM judge | [C] |
| Others [186, 187, 188, 189] | Associative and hierarchical memory | CAMELoT, Memory Mosaics, RAPTOR, MIRIX | Various | — | [C] |

**Benchmarks and critiques:**
- **MemoryAgentBench:** "all methods fail on the multi-hop" conflict-resolution items (≤28%) [I, 190].
- **A frozen last-write rule** answers 80.25% of those conflict-resolution items (74.5% held out) [I/C, 191]: bookkeeping beats learned forgetting.
- **VibeMemBench:** directly injecting verified experience raised resolution by 1.1–4.5 percentage points on 4 of 5 solvers and cut steps on all 5. But 11 of 12 system/solver pairings failed to beat memory-off [I, 192].
- **Position papers and surveys:** retrieval is "lookup, not memory", with poisoning that propagates [194]; arguments for episodic and explicit memory [193, 197]; surveys [195, 196].

**Overall reading [Inf].** Reported gains come from three sources:
1. better *keys and indexes* (entities, graphs, segments), mostly for multi-hop questions;
2. *selective, structured writing* (segmentation, failure-driven);
3. *consolidation with explicit applicability* that prevents accumulation decay.

There is no evidence that biological fidelity itself matters.

---

## 4. Part C — Mapping to hivemind

### 4.1 Mapping table

| ID | Neuroscience concept → computational principle | Concrete hivemind mechanism | AI evidence | Cost | Verdict |
|---|---|---|---|---|---|
| C1 | Indexing / key–value → keys tuned for discriminability, values for fidelity | Cards are keys; episodes and artifacts are values; deterministic **key expansion** at write (error codes, normalised messages, paths, symbols, package@version, applicability) | HippoRAG [C]; LongMemEval key expansion [V]; GraphRAG-Bench [I] | Low | **v0** |
| C2 | Event boundaries → segment at prediction error or context shift; encode at boundary | Episode boundaries from deterministic signals; **boundary write hook** emits an episode card | SeCom, EM-LLM [C]; LongMemEval decomposition [V]; finer chunks help [I, 190] | Low | **v0** |
| C3 | Novelty / prediction error → graded encoding strength | `salience` from mismatch signals (non-zero exit, test fail, expected ≠ observed, first-seen error signature, claim contradicted); raises ranking prior, retention class and write prompts; episodes always kept | Hippo, Reflexion, ReasoningBank [C]; Titans [C/I] | Low | **v0 weighting**; gating rejected pending E3 |
| C4 | CA3 completion → spreading activation from partial cues | Optional `expand=assoc`: PPR over entity–claim–episode graph with typed edge weights; returns "why" paths | HippoRAG/2 [C]; graph helps complex, hurts simple [I] | Med | **v1** (flagged) |
| C5 | Dentate-gyrus separation and learned repulsion → keep similar but different memories apart | Near-duplicate detection (SimHash/MinHash plus vector) → *corroborate* or *separate*; separation requires a discriminating applicability field and a `contrasts_with` edge; cards show "differs in: …" | Hippo applicability [C]; conflict benchmarks [I] | Low | **v0** |
| C6 | Reconsolidation / latent cause → update only when cause matches | `feedback(claim, used/helpful/not_applicable/wrong, evidence)` events; `wrong`+evidence → **contested**; supersede only if applicability matches, else sibling claim; no in-place mutation | Copilot vote/validation (research/04) [C]; A-MEM [C] | Low | **v0** |
| C7 | Prioritised replay (gain × need) and ripple tagging | Consolidation queue under a token budget: priority = gain (contradictions, new failures, unconsolidated clusters, negative feedback) × need (accessibility of involved entities, open work items) / token cost | PER [C/I]; sleep-time compute [C]; LightMem [C] | Med | **v1** |
| C8 | Concept cells → sparse, explicit entity nodes; fast association | Canonical entity table (file, symbol, package@range, normalised error, command, service, agent, repo); `about` edges; co-mention edges | HippoRAG nodes [C]; Zep [C] | Low | **v0** |
| C9 | Temporal context / contiguity → neighbours, forward bias | `expand(card, neighbors=±k, direction=forward)` over the ordered log; small ranking boost for items temporally adjacent to strong hits | EM-LLM [C]; HippoMM [C]; [V, 86] | Very low | **v0** |
| C10 | Successor representation → predictive transitions | Offline counts "after error-class X in context C, next action Y succeeded k/n", returned as a suggestion card with evidence links | AWM, ExpeL [C] (indirect) | Low–Med | **v1** |
| C11 | Engram linking in time; retrospective linking | Weak `co_temporal` edges within an episode or work item; on salient failure, back-link to the previous N minutes of the same agent or work item | None [?] | Very low | **v1** |
| C12 | Schemas → fast track for congruent, slow path for novel | `instance_of` attach (counts, no LLM) when a claim matches an existing procedure or error family; novel claims start *provisional* with higher evidence requirements; contradicting a strong schema raises gain for C7 | None direct [?] | Low | **v1** |
| C13 | Go-CLS → consolidate only what generalises | Promote a consolidated claim to trusted only if it predicts outcomes on *held-out* episodes | Hippo inverted-U [C] | Med | **v1 (high priority)** |
| C14 | Adaptive forgetting / rational analysis → accessibility tracks need | ACT-R-style base-level activation from access and helpful events, per-kind decay; superseded or expired hidden by default; decay never deletes | Last-write [I]; MemoryBank, HEMA [C] | Low | **v0** |
| C15 | Source monitoring → explicit source tags | Mandatory provenance plus `method` (observed / inferred / consolidated / imported); cards display source and verification; consolidated chains must bottom out in evidence | Copilot citations (research/04) [C] | Low | **v0** (core) |
| C16 | Generative consolidation / compression → cortex learns the predictable, hippocampus keeps residuals | Summaries and procedures as "cortex"; outputs must state what is non-derivable; detail requests always go to evidence; never regenerate evidence | RAPTOR, AWM, ACE [C] | High | **v1** |
| C17 | REMERGE → generalisation at retrieval | On-demand aggregation card over retrieved episodes (cluster plus counts), cached | Theory only | Low–Med | **v1** (baseline for C16) |
| C18 | Selective retrieval (Lu et al.) → retrieve only when uncertain | Abstain below a score threshold; push only at boundary events (session start, failure) | [V, 78]; context-file costs (research/04) [I] | Low | **v0** |
| C19 | TEM / composition → factor structure from specifics | `procedure` claims with typed slots ({pkg}, {version}, {os}) plus instance bindings | AWM abstracted workflows [C] | Med | **v1** |

### 4.2 Per-mechanism notes and falsifiable experiments
All experiments run on hivebench (docs/05, D1) with the token ledger. Thresholds are pre-registered hypotheses, not promises.

- **C1 Key–value cards.**
  - *Principle:* keys and values have different jobs [8].
  - *Mechanism:* agent-written card plus deterministic extraction (regex or tree-sitter), no LLM.
  - *Experiment E1 (extends F4/F7):* index (a) raw episode text, (b) card text, (c) card + expanded keys.
  - *Metrics:* recall@5 and tokens per useful hit.
  - *Prediction:* (c) beats (a) by ≥5 points recall@5 at ≤ equal tokens. *Falsified* if (a) ≥ (c).
- **C2 Boundary episodes.**
  - *Signals:* session start/end, work-item change, branch or commit, test status flip (fail→pass is a strong "solution" boundary), cwd/repo switch, tool-error burst, idle gap > T.
  - At each boundary the writer is prompted for an episode card (goal, actions, outcome, artifacts), mirroring boundary reinstatement [74]. Encoding at boundaries outperformed mid-event encoding in a model [78].
  - *Experiment E2:* fixed-size chunks vs whole session vs boundary episodes.
  - *Metrics:* precision@5, tokens per hit, task success.
  - *Prediction:* boundary episodes give ≥10% relative precision gain and fewer tokens per hit.
  - *Secondary prediction (from [75]):* cross-boundary order questions get worse unless C9 links exist.
- **C3 Salience weighting.**
  - *Experiment E3 (extends F6):* write policy (a) free writes, (b) surprise-gated only, (c) all writes + salience weighting.
  - *Metrics:* memory volume; success on recurring-failure probes; **miss rate on "boring-but-critical" probes** (build or test commands, conventions).
  - *Prediction:* (c) ≥ (a) on success; (b) halves volume but raises miss rate by more than 5 points. That would confirm rejecting gating.
  - *Falsified* if (b) matches (c) on all probes.
- **C4 PPR completion.**
  - *Cost:* PPR power iteration on a laptop-scale graph should be milliseconds in Rust [Inf]. The real cost is graph upkeep and spurious-edge risk.
  - *Experiment E4 (extends F10):* error-string → fix probes where the fix shares no tokens with the error.
  - *Prediction:* BM25 + vector + PPR fused improves recall@10 by ≥10 points on indirect or multi-hop probes and costs ≤2 points on single-hop. Otherwise route PPR only to multi-hop queries or drop it.
- **C5 Separation.**
  - *Experiment E5:* inject near-duplicate fixes differing only by applicability (OS or version). Compare (a) dedup-merge, (b) keep-all without discriminators, (c) separation with contrast edges.
  - *Metric:* wrong-variant application rate.
  - *Prediction:* (c) is ≥50% lower than (a).
  - *Falsified* if (b) ≈ (c), meaning discriminators add nothing.
- **C6 Feedback / latent cause.**
  - *Rule:* same applicability → `supersede`; different → new sibling with `contrasts_with`; unclear → `contested`. Positive feedback strengthens only with evidence, avoiding popularity loops.
  - *Experiment E6 (extends F5):* a dependency upgrade invalidates a stored fix.
  - *Metrics:* sessions until the stale claim is demoted; repeated-stale-application rate.
  - *Prediction:* the feedback loop cuts repeats by ≥50% vs no feedback.
- **C7 Gain × need scheduler.**
  - *Need* is estimated from C14 activation of the involved entities plus open work items. *Gain* comes from deterministic triggers.
  - *Experiment E7:* fixed background budget B; compare FIFO, random, recency, gain×need.
  - *Metric:* downstream probe success per consolidation token.
  - *Prediction:* gain×need gives ≥1.5× FIFO. *Falsified* if within noise.
  - Sleep-time compute suggests benefits concentrate where queries are predictable [162].
- **C8 Entity index.**
  - *Experiment E8:* entity boost on/off for queries containing a normalised entity.
  - *Prediction:* ≥10 points recall@5 gain. Measure entity-extraction precision ≥95% on a hand-audited sample.
- **C9 Temporal neighbours.**
  - *Experiment E9:* given the retrieved error episode, does forward expansion put the fix within the top 3?
  - *Prediction:* ≥15 points more often than similarity alone. Nearly free because the log is ordered.
- **C10 Successor links.**
  - *Experiment E10:* next-action suggestion vs the single most similar episode.
  - *Prediction:* aggregate counts win when ≥5 prior episodes exist; no difference below that.
  - Report counts and evidence, never bare advice.
- **C11 Co-temporal linking.**
  - *Experiment E11:* root-cause probes where the cause was an earlier change by the same agent or work item.
  - *Prediction:* retro-links surface the cause in the top 5 more often.
  - *Guard:* scope to the same agent or work item. Interleaved multi-agent timing is not relatedness.
- **C12 Schema fast-track.**
  - *Experiment E12:* audit precision of auto `instance_of` (target ≥90%).
  - Measure *suppression* of valid incongruent claims, which is the confirmation-bias risk.
- **C13 Selective consolidation.**
  - *Experiment E13:* consolidate-all vs consolidate-if-predictive (leave-episodes-out validation of each candidate procedure's applicability → outcome).
  - Track success and wrong-application rate **as memory grows**.
  - *Prediction:* consolidate-all shows the inverted U [182]; selective does not.
- **C14 Accessibility decay.**
  - *Experiment E14 (extends F5):* drifted long-horizon corpus.
  - *Metrics:* stale-hit rate; recall of rare-but-valid claims (for example a yearly release procedure).
  - *Prediction:* stale hits drop ≥30% with ≤5 points loss on rare-valid recall. If the loss is larger, make decay strictly kind-specific.
- **C15 Provenance / source monitoring.**
  - *Experiment E15 (extends F12):* claims whose evidence does not support them.
  - *Metric:* agent action rate with vs without source display and a verification step.
  - *Prediction:* substantial reduction. Consolidated claims must never outrank their evidence.
- **C16 vs C17.**
  - *Experiment E16:* (a) episodes only, (b) summaries only, (c) summaries + episode index, (d) on-demand aggregation (C17).
  - *Metrics:* correct answers per token; detail-question accuracy.
  - *Prediction:* (c) best overall; (b) loses ≥10 points on detail questions (context collapse [168]); (d) is competitive at lower background cost.
- **C18 Abstention.**
  - Tie to F3/F7: measure the irrelevant-injection rate and tokens with thresholded abstention vs always-retrieve.
- **C19 Slotted procedures.**
  - *Experiment E19:* cross-repo transfer probes.
  - *Prediction:* slotted procedures beat concrete instances in transfer success at equal tokens.

### 4.3 Rejected as metaphor without engineering payoff
- **R1. Hopfield, SDM or dense associative memory as the store.** No evidence it beats BM25 plus approximate nearest-neighbour search for text claims at laptop scale. It is opaque and hard to audit. Keep only as design lessons (A18).
- **R2. Test-time weight memory (Titans, Larimar, AHN).** Requires model internals; hivemind serves black-box agents. Even the Titans reimplementation shows mixed gains [150].
- **R3. Generative replay.** Solves catastrophic forgetting in weights. hivemind stores exemplars and has no weights to protect.
- **R4. Neurogenesis-style random forgetting or noise.** Human evidence is debated (A3), and the forgetting it would produce cannot be audited.
- **R5. Literal sleep cycles or oscillations.** Scheduling is a budget and priority problem (C7), not a clock.
- **R6. Generative "reconstruction" of evidence.** Conflicts with provenance; consolidation already introduces schema distortions [124].
- **R7. Emotional or fear analogues.** Nothing beyond "salience" (C3).
- **R8. Pure surprise gating of writes.** Replaced by weighting unless E3 shows otherwise.
- **R9. Standard-consolidation deletion of episodes after consolidation.** Contradicts transformation and contextual-binding accounts (A5) and provenance.

---

## 5. Part D — Recommendation and risks

**Build into v0** (cheap and deterministic, each with an experiment):
1. C1 key–value cards with key expansion, plus C8 entity index.
2. C2 boundary-segmented episodes with a boundary write hook.
3. C9 temporal-neighbour expansion with forward bias.
4. C3 salience *weighting* from deterministic mismatch signals.
5. C6 retrieval feedback, a `contested` state, and the latent-cause supersede rule.
6. C5 near-duplicate separation with discriminators.
7. C14 ACT-R-style, kind-specific accessibility decay (never deletion).
8. C15 provenance and source display.
9. C18 abstention, with push only at boundaries.

**Test in v1, in priority order:**
1. C13 selective consolidation;
2. C7 gain×need scheduler;
3. C4 PPR completion;
4. C16 vs C17;
5. C19 slotted procedures;
6. C10 successor links;
7. C12 schema fast-track;
8. C11 co-temporal linking.

**Reject:** R1–R9.

**Risks of over-applying the analogy:**
- **Theory risk.** Several source theories are themselves contested: consolidation flavour, human reconsolidation, replay-as-planning, adult neurogenesis. A mechanism must earn its place on hivebench, not on biological plausibility.
- **Evidence risk.** Most AI results are author-run on chat benchmarks (LoCoMo, LongMemEval), not coding. Independent coding evidence is mostly negative [192].
- **Loss risk.** Surprise gating or aggressive decay drops boring-but-critical knowledge (C3, C14).
- **Distortion risk.** Consolidation and schema fast-tracking introduce gist errors and confirmation bias [53, 124]; iterative rewriting collapses detail [168].
- **Security risk.** Spreading activation (C4), co-temporal links (C11) and salience boosts (C3) can amplify poisoned or spurious memories. Trust must propagate no higher than the weakest edge (docs/03).
- **Complexity risk.** Every added edge type and offline job costs laptop resources and maintainer attention. Default off until it shows measured lift.
- **Multi-agent mismatch.** The brain is one agent. Time proximity and "surprise" across interleaved agents are not the same signals, so scope them per agent or work item.

## 6. Open questions
1. Which boundary signals best segment coding sessions? Does LLM-estimated surprise add anything over deterministic signals (E2)?
2. Can "need" be predicted cheaply from repository activity and open work items well enough to schedule consolidation (E7)?
3. How can "generalises" be tested cheaply for Go-CLS gating when episodes are few (E13)?
4. Given VibeMemBench [192], is consolidation ever net-positive for coding agents? Hippo [182] says yes with merge-plus-applicability; this needs independent replication.
5. Does PPR beat simply letting the coding agent iterate retrieval itself, as IRCoT does? HippoRAG's cost advantage was measured against iterative retrieval [142].
6. Salience across agents: whose surprise counts, and can salience be gamed by a poisoning writer?
7. Do discriminator prompts (C5) and feedback (C6) shift too much burden onto writing agents? This interacts with F6.
8. Is contiguity (C9) still useful when sessions interleave many tools and subagents?

## 7. Sources (all checked 2026-11-10)

**Neuroscience and cognitive science**
1. McClelland, McNaughton & O'Reilly (1995) Psychol Rev 102:419–457 — https://doi.org/10.1037/0033-295X.102.3.419
2. Kumaran, Hassabis & McClelland (2016) Trends Cogn Sci 20:512–534 — https://doi.org/10.1016/j.tics.2016.05.004
3. Sun, Advani, Spruston, Saxe & Fitzgerald (2023) Nat Neurosci 26:1438–1448 — https://doi.org/10.1038/s41593-023-01382-9
4. Schapiro, Turk-Browne, Botvinick & Norman (2017) Phil Trans R Soc B 372:20160049 — https://doi.org/10.1098/rstb.2016.0049
5. Kumaran & McClelland (2012) Psychol Rev 119:573–616 — https://doi.org/10.1037/a0028681
6. Teyler & DiScenna (1986) Behav Neurosci 100:147–154 — https://doi.org/10.1037/0735-7044.100.2.147
7. Teyler & Rudy (2007) Hippocampus 17:1158–1169 — https://doi.org/10.1002/hipo.20350
8. Gershman, Fiete & Irie (2025) Neuron 113:1694–1707 — https://doi.org/10.1016/j.neuron.2025.02.029 (arXiv 2501.02950)
9. Marr (1971) Phil Trans R Soc B 262:23–81 — https://doi.org/10.1098/rstb.1971.0078
10. Treves & Rolls (1994) Hippocampus 4:374–391 — https://doi.org/10.1002/hipo.450040319
11. Rolls (2013) Front Syst Neurosci 7:74 — https://doi.org/10.3389/fnsys.2013.00074
12. Yassa & Stark (2011) Trends Neurosci 34:515–525 — https://doi.org/10.1016/j.tins.2011.06.006
13. Leutgeb et al. (2007) Science 315:961–966 — https://doi.org/10.1126/science.1135801
14. Bakker, Kirwan, Miller & Stark (2008) Science 319:1640–1642 — https://doi.org/10.1126/science.1152882
15. Nakazawa et al. (2002) Science 297:211–218 — https://doi.org/10.1126/science.1071795
16. Amer & Davachi (2023) eLife 12:e82250 — https://doi.org/10.7554/eLife.82250
17. Sahay et al. (2011) Nature 472:466–470 — https://doi.org/10.1038/nature09817
18. Sorrells et al. (2018) Nature 555:377–381 — https://doi.org/10.1038/nature25975
19. Boldrini et al. (2018) Cell Stem Cell 22:589–599 — https://doi.org/10.1016/j.stem.2018.03.015
20. Moreno-Jiménez et al. (2019) Nat Med 25:554–560 — https://doi.org/10.1038/s41591-019-0375-9
21. Dumitru et al. (2025) Science 389:58–63 — https://doi.org/10.1126/science.adu9575
22. Favila, Chanales & Kuhl (2016) Nat Commun 7:11066 — https://doi.org/10.1038/ncomms11066
23. Chanales, Oza, Favila & Kuhl (2017) Curr Biol 27:2307–2317 — https://doi.org/10.1016/j.cub.2017.06.057
24. Ritvo, Turk-Browne & Norman (2019) Trends Cogn Sci 23:726–742 — https://doi.org/10.1016/j.tics.2019.06.007
25. Wu & Maass (2025) Nat Commun 16 — https://doi.org/10.1038/s41467-024-55563-6
26. Dasgupta, Stevens & Navlakha (2017) Science 358:793–796 — https://doi.org/10.1126/science.aam9868
27. Lisman & Grace (2005) Neuron 46:703–713 — https://doi.org/10.1016/j.neuron.2005.05.002
28. Duncan, Ketz, Inati & Davachi (2012) Hippocampus 22:389–398 — https://doi.org/10.1002/hipo.20933
29. Kumaran & Maguire (2007) Hippocampus 17:735–748 — https://doi.org/10.1002/hipo.20326
30. Takeuchi et al. (2016) Nature 537:357–362 — https://doi.org/10.1038/nature19325
31. Duszkiewicz, McNamara, Takeuchi & Genzel (2019) Trends Neurosci 42:102–114 — https://doi.org/10.1016/j.tins.2018.10.002
32. Rouhani, Norman & Niv (2018) J Exp Psychol LMC 44:1430–1443 — https://doi.org/10.1037/xlm0000518
33. Rouhani, Norman, Niv & Bornstein (2020) Cognition 203:104269 — https://doi.org/10.1016/j.cognition.2020.104269
34. Greve, Cooper, Kaula, Anderson & Henson (2017) J Mem Lang 94:149–165 — https://doi.org/10.1016/j.jml.2016.11.001
35. Hasselmo (2006) Curr Opin Neurobiol 16:710–715 — https://doi.org/10.1016/j.conb.2006.09.002
36. Squire & Alvarez (1995) Curr Opin Neurobiol 5:169–177 — https://doi.org/10.1016/0959-4388(95)80023-9
37. Nadel & Moscovitch (1997) Curr Opin Neurobiol 7:217–227 — https://doi.org/10.1016/S0959-4388(97)80010-4
38. Winocur & Moscovitch (2011) J Int Neuropsychol Soc 17:766–780 — https://doi.org/10.1017/S1355617711000683
39. Sekeres, Winocur & Moscovitch (2018) Neurosci Lett 680:39–53 — https://doi.org/10.1016/j.neulet.2018.05.006
40. Frankland & Bontempi (2005) Nat Rev Neurosci 6:119–130 — https://doi.org/10.1038/nrn1607
41. Kitamura et al. (2017) Science 356:73–78 — https://doi.org/10.1126/science.aam6808
42. Yonelinas, Ranganath, Ekstrom & Wiltgen (2019) Nat Rev Neurosci 20:364–375 — https://doi.org/10.1038/s41583-019-0150-4
43. Tomé et al. (2024) Nat Neurosci — https://doi.org/10.1038/s41593-023-01551-w (PMID 38243089)
44. Nader, Schafe & LeDoux (2000) Nature 406:722–726 — https://doi.org/10.1038/35021052
45. Sevenster, Beckers & Kindt (2013) Science 339:830–833 — https://doi.org/10.1126/science.1231357
46. Sinclair & Barense (2019) Trends Neurosci 42:727–739 — https://doi.org/10.1016/j.tins.2019.08.007
47. Fernández, Boccia & Pedreira (2016) Neurosci Biobehav Rev 68:423–441 — https://doi.org/10.1016/j.neubiorev.2016.06.004
48. Gershman, Monfils, Norman & Niv (2017) eLife 6:e23763 — https://doi.org/10.7554/eLife.23763
49. Hardwicke, Taqi & Shanks (2016) PNAS 113:5206–5211 — https://doi.org/10.1073/pnas.1601440113
50. Elsey, Van Ast & Kindt (2018) Psychol Bull 144:797–848 — https://doi.org/10.1037/bul0000152
51. Tse et al. (2007) Science 316:76–82 — https://doi.org/10.1126/science.1135935
52. van Kesteren, Ruiter, Fernández & Henson (2012) Trends Neurosci 35:211–219 — https://doi.org/10.1016/j.tins.2012.02.001
53. Gilboa & Marlatte (2017) Trends Cogn Sci 21:618–631 — https://doi.org/10.1016/j.tics.2017.04.013
54. Wilson & McNaughton (1994) Science 265:676–679 — https://doi.org/10.1126/science.8036517
55. Foster & Wilson (2006) Nature 440:680–683 — https://doi.org/10.1038/nature04587
56. Pfeiffer & Foster (2013) Nature 497:74–79 — https://doi.org/10.1038/nature12112
57. Ambrose, Pfeiffer & Foster (2016) Neuron 91:1124–1136 — https://doi.org/10.1016/j.neuron.2016.07.047
58. Girardeau et al. (2009) Nat Neurosci 12:1222–1223 — https://doi.org/10.1038/nn.2384
59. Fernández-Ruiz et al. (2019) Science 364:1082–1086 — https://doi.org/10.1126/science.aax0758
60. Buzsáki (2015) Hippocampus 25:1073–1188 — https://doi.org/10.1002/hipo.22488
61. Joo & Frank (2018) Nat Rev Neurosci 19:744–757 — https://doi.org/10.1038/s41583-018-0077-1
62. Mattar & Daw (2018) Nat Neurosci 21:1609–1617 — https://doi.org/10.1038/s41593-018-0232-z (full text PMC6203620)
63. Liu, Mattar, Behrens, Daw & Dolan (2021) Science — https://doi.org/10.1126/science.abf1357
64. Gillespie et al. (2021) Neuron 109:3149–3163 — https://doi.org/10.1016/j.neuron.2021.07.029
65. Yang et al. (2024) Science 383:1478–1483 — https://doi.org/10.1126/science.adk8261
66. Eldar, Lièvre, Dayan & Dolan (2020) eLife — https://doi.org/10.7554/eLife.56911
67. Liu, Dolan, Kurth-Nelson & Behrens (2019) Cell 178:640–652 — https://doi.org/10.1016/j.cell.2019.06.012
68. Zacks et al. (2007) Psychol Bull 133:273–293 — https://doi.org/10.1037/0033-2909.133.2.273
69. Ezzyat & Davachi (2011) Psychol Sci 22:243–252 — https://doi.org/10.1177/0956797610393742
70. DuBrow & Davachi (2013) J Exp Psychol Gen 142:1277–1286 — https://doi.org/10.1037/a0034024
71. DuBrow & Davachi (2016) Neurobiol Learn Mem 134:107–114 — https://doi.org/10.1016/j.nlm.2016.07.011
72. Ben-Yakov & Henson (2018) J Neurosci 38:10057–10068 — https://doi.org/10.1523/JNEUROSCI.0524-18.2018
73. Baldassano et al. (2017) Neuron 95:709–721 — https://doi.org/10.1016/j.neuron.2017.06.041
74. Sols, DuBrow, Davachi & Fuentemilla (2017) Curr Biol 27:3499–3504 — https://doi.org/10.1016/j.cub.2017.09.057
75. Zheng et al. (2022) Nat Neurosci 25:358–368 — https://doi.org/10.1038/s41593-022-01020-w
76. Clewett, DuBrow & Davachi (2019) Hippocampus 29:162–183 — https://doi.org/10.1002/hipo.23074
77. Franklin, Norman, Ranganath, Zacks & Gershman (2020) Psychol Rev 127:327–361 — https://doi.org/10.1037/rev0000177
78. Lu, Hasson & Norman (2022) eLife 11:e74445 — https://doi.org/10.7554/eLife.74445
79. Howard & Kahana (2002) J Math Psychol 46:269–299 — https://doi.org/10.1006/jmps.2001.1388
80. Kahana (1996) Mem Cogn 24:103–109 — https://doi.org/10.3758/BF03197276
81. Polyn, Norman & Kahana (2009) Psychol Rev 116:129–156 — https://doi.org/10.1037/a0014420
82. Healey, Long & Kahana (2019) Psychon Bull Rev 26:699–720 — https://doi.org/10.3758/s13423-018-1537-3
83. MacDonald, Lepage, Eden & Eichenbaum (2011) Neuron 71:737–749 — https://doi.org/10.1016/j.neuron.2011.07.012
84. Umbach et al. (2020) PNAS 117:28463–28474 — https://doi.org/10.1073/pnas.2013250117
85. Tacikowski, Kalender, Ciliberti & Fried (2024) Nature 635:160–167 — https://doi.org/10.1038/s41586-024-07973-1
86. Ji-An, Zhou, Benna & Mattar (2024) NeurIPS — https://arxiv.org/abs/2405.14992
87. Dayan (1993) Neural Comput 5:613–624 — https://doi.org/10.1162/neco.1993.5.4.613
88. Stachenfeld, Botvinick & Gershman (2017) Nat Neurosci 20:1643–1653 — https://doi.org/10.1038/nn.4650
89. Momennejad et al. (2017) Nat Hum Behav 1:680–692 — https://doi.org/10.1038/s41562-017-0180-8
90. Garvert, Dolan & Behrens (2017) eLife 6:e17086 — https://doi.org/10.7554/eLife.17086
91. Whittington et al. (2020) Cell 183:1249–1263 — https://doi.org/10.1016/j.cell.2020.10.024
92. Whittington, Warren & Behrens (2022) ICLR — https://arxiv.org/abs/2112.04035
93. George et al. (2021) Nat Commun 12 — https://doi.org/10.1038/s41467-021-22559-5
94. Raju et al. (2024) Sci Adv 10 — https://doi.org/10.1126/sciadv.adm8470
95. Whittington, Dorrell, Behrens, Ganguli & El-Gaby (2025) Neuron 113:321–333 — https://doi.org/10.1016/j.neuron.2024.10.017
96. Bakermans, Warren, Whittington & Behrens (2025) Nat Neurosci 28:1061–1072 — https://doi.org/10.1038/s41593-025-01908-3
97. Schwartenbeck et al. (2023) Cell 186:4885–4897 — https://doi.org/10.1016/j.cell.2023.09.004
98. Kurth-Nelson et al. (2023) Neuron 111:454–469 — https://doi.org/10.1016/j.neuron.2022.12.028
99. Quian Quiroga, Reddy, Kreiman, Koch & Fried (2005) Nature 435:1102–1107 — https://doi.org/10.1038/nature03687
100. Quian Quiroga (2012) Nat Rev Neurosci 13:587–597 — https://doi.org/10.1038/nrn3251
101. Ison, Quian Quiroga & Fried (2015) Neuron 87:220–230 — https://doi.org/10.1016/j.neuron.2015.06.016
102. Behrens et al. (2018) Neuron 100:490–509 — https://doi.org/10.1016/j.neuron.2018.10.002
103. Constantinescu, O'Reilly & Behrens (2016) Science 352:1464–1468 — https://doi.org/10.1126/science.aaf0941
104. Han et al. (2007) Science 316:457–460 — https://doi.org/10.1126/science.1139438
105. Cai et al. (2016) Nature 534:115–118 — https://doi.org/10.1038/nature17955
106. Rashid et al. (2016) Science 353:383–387 — https://doi.org/10.1126/science.aaf0594
107. Yokose et al. (2017) Science 355:398–403 — https://doi.org/10.1126/science.aal2690
108. Zaki et al. (2025) Nature 637:145–155 — https://doi.org/10.1038/s41586-024-08168-4
109. Josselyn & Tonegawa (2020) Science 367:eaaw4325 — https://doi.org/10.1126/science.aaw4325
110. Richards & Frankland (2017) Neuron 94:1071–1084 — https://doi.org/10.1016/j.neuron.2017.04.037
111. Davis & Zhong (2017) Neuron 95:490–503 — https://doi.org/10.1016/j.neuron.2017.05.039
112. Ryan & Frankland (2022) Nat Rev Neurosci 23:173–186 — https://doi.org/10.1038/s41583-021-00548-3
113. Anderson & Hulbert (2021) Annu Rev Psychol 72:1–36 — https://doi.org/10.1146/annurev-psych-072720-094140
114. Anderson, Bjork & Bjork (1994) J Exp Psychol LMC 20:1063–1087 — https://doi.org/10.1037/0278-7393.20.5.1063
115. Akers et al. (2014) Science 344:598–602 — https://doi.org/10.1126/science.1248903
116. Anderson & Schooler (1991) Psychol Sci 2:396–408 — https://doi.org/10.1111/j.1467-9280.1991.tb00174.x
117. Anderson et al. (2004) Psychol Rev 111:1036 — https://doi.org/10.1037/0033-295X.111.4.1036
118. Roediger & Karpicke (2006) Psychol Sci 17:249–255 — https://doi.org/10.1111/j.1467-9280.2006.01693.x
119. Antony, Ferreira, Norman & Wimber (2017) Trends Cogn Sci 21:573–576 — https://doi.org/10.1016/j.tics.2017.05.001
120. Johnson, Hashtroudi & Lindsay (1993) Psychol Bull 114:3–28 — https://doi.org/10.1037/0033-2909.114.1.3
121. Schacter (1999) Am Psychol 54:182–203 — https://doi.org/10.1037/0003-066X.54.3.182
122. Schacter & Addis (2007) Phil Trans R Soc B 362:773–786 — https://doi.org/10.1098/rstb.2007.2087
123. Roediger & McDermott (1995) J Exp Psychol LMC 21:803–814 — https://doi.org/10.1037/0278-7393.21.4.803
124. Spens & Burgess (2024) Nat Hum Behav 8:526–543 — https://doi.org/10.1038/s41562-023-01799-z
125. Nagy, Török & Orbán (2020) PLoS Comput Biol 16:e1008367 — https://doi.org/10.1371/journal.pcbi.1008367
126. Benna & Fusi (2016) Nat Neurosci 19:1697–1706 — https://doi.org/10.1038/nn.4401
127. Fusi, Drew & Abbott (2005) Neuron 45:599–611 — https://doi.org/10.1016/j.neuron.2005.02.001
128. Benna & Fusi (2021) PNAS 118 — https://doi.org/10.1073/pnas.2018422118
129. Hopfield (1982) PNAS 79:2554–2558 — https://doi.org/10.1073/pnas.79.8.2554
130. Kanerva (1988) *Sparse Distributed Memory*, MIT Press, ISBN 9780262111324 — https://openlibrary.org/search?q=sparse+distributed+memory+kanerva
131. Krotov & Hopfield (2016) NeurIPS — https://arxiv.org/abs/1606.01164
132. Demircigil et al. (2017) J Stat Phys 168:288–299 — https://doi.org/10.1007/s10955-017-1806-y
133. Ramsauer et al. (2020) — https://arxiv.org/abs/2008.02217
134. Bricken & Pehlevan (2021) NeurIPS — https://arxiv.org/abs/2111.05498
135. Chandra, Sharma, Chaudhuri & Fiete (2025) Nature 638:739–751 — https://doi.org/10.1038/s41586-024-08392-y
136. Bittner et al. (2017) Science 357:1033–1036 — https://doi.org/10.1126/science.aan3846
137. Priestley et al. (2022) Neuron 110:1978–1992 — https://doi.org/10.1016/j.neuron.2022.03.026
138. Levenstein et al. (2024) bioRxiv preprint — https://doi.org/10.1101/2024.04.28.591528
139. Kahana (2020) Annu Rev Psychol 71:107–138 — https://doi.org/10.1146/annurev-psych-010418-103358
140. Diekelmann & Born (2010) Nat Rev Neurosci 11:114–126 — https://doi.org/10.1038/nrn2762
141. Gershman & Daw (2017) Annu Rev Psychol 68:101–128 — https://doi.org/10.1146/annurev-psych-122414-033625

**AI / ML**

142. Gutiérrez et al. (2024) HippoRAG, NeurIPS — https://arxiv.org/abs/2405.14831
143. Gutiérrez et al. (2025) HippoRAG 2, ICML — https://arxiv.org/abs/2502.14802
144. Xiang et al. (2025) GraphRAG-Bench — https://arxiv.org/abs/2506.05690
145. Han et al. (2025) RAG vs GraphRAG — https://arxiv.org/abs/2502.11371
146. Fountas et al. (2025) EM-LLM, ICLR — https://arxiv.org/abs/2407.09450
147. Das et al. (2024) Larimar, ICML — https://arxiv.org/abs/2403.11901
148. Wu, Wayne, Graves & Lillicrap (2018) Kanerva Machine — https://arxiv.org/abs/1804.01756
149. Behrouz, Zhong & Mirrokni (2024) Titans — https://arxiv.org/abs/2501.00663
150. Di Nepi, Siciliano & Silvestri (2025) Titans Revisited — https://arxiv.org/abs/2510.09551
151. Lin (1992) Mach Learn 8:293–321 — https://doi.org/10.1023/A:1022628806385
152. Mnih et al. (2015) Nature 518:529–533 — https://doi.org/10.1038/nature14236
153. Schaul, Quan, Antonoglou & Silver (2016) Prioritized Experience Replay, ICLR — https://arxiv.org/abs/1511.05952
154. Hessel et al. (2017) Rainbow — https://arxiv.org/abs/1710.02298
155. Shin, Lee, Kim & Kim (2017) Deep Generative Replay, NIPS — https://arxiv.org/abs/1705.08690
156. van de Ven, Siegelmann & Tolias (2020) Nat Commun 11 — https://doi.org/10.1038/s41467-020-17866-2
157. van de Ven, Tuytelaars & Tolias (2022) Nat Mach Intell 4:1185–1197 — https://doi.org/10.1038/s42256-022-00568-3
158. Tadros, Krishnan, Ramyaa & Bazhenov (2022) Nat Commun 13 — https://doi.org/10.1038/s41467-022-34938-7
159. Arani, Sarfraz & Zonooz (2022) CLS-ER — https://arxiv.org/abs/2201.12604
160. Blundell et al. (2016) Model-Free Episodic Control — https://arxiv.org/abs/1606.04460
161. Pritzel et al. (2017) Neural Episodic Control — https://arxiv.org/abs/1703.01988
162. Lin et al. (2025) Sleep-time Compute — https://arxiv.org/abs/2504.13171
163. Park et al. (2023) Generative Agents — https://arxiv.org/abs/2304.03442
164. Shinn et al. (2023) Reflexion — https://arxiv.org/abs/2303.11366
165. Zhao et al. (2023) ExpeL — https://arxiv.org/abs/2308.10144
166. Wang, Mao, Fried & Neubig (2024) Agent Workflow Memory — https://arxiv.org/abs/2409.07429
167. Ouyang et al. (2025) ReasoningBank, ICLR 2026 — https://arxiv.org/abs/2509.25140
168. Zhang et al. (2025) ACE — https://arxiv.org/abs/2510.04618
169. Suzgun et al. (2025) Dynamic Cheatsheet — https://arxiv.org/abs/2504.07952
170. Xu et al. (2025) A-MEM, NeurIPS — https://arxiv.org/abs/2502.12110
171. Chhikara et al. (2025) Mem0 — https://arxiv.org/abs/2504.19413
172. Rasmussen et al. (2025) Zep — https://arxiv.org/abs/2501.13956
173. Zhong et al. (2023) MemoryBank — https://arxiv.org/abs/2305.10250
174. Wu et al. (2025) LongMemEval, ICLR — https://arxiv.org/abs/2410.10813
175. Maharana et al. (2024) LoCoMo — https://arxiv.org/abs/2402.17753
176. Pan et al. (2025) SeCom — https://arxiv.org/abs/2502.05589
177. Fang et al. (2025) LightMem, ICLR 2026 — https://arxiv.org/abs/2510.18866
178. Ma et al. (2025) Nemori / "What Deserves Memory" — https://arxiv.org/abs/2508.03341
179. Lin et al. (2025) HippoMM — https://arxiv.org/abs/2504.10739
180. Ahn (2025) HEMA — https://arxiv.org/abs/2504.16754
181. Li et al. (2026) Hippocampus memory module — https://arxiv.org/abs/2602.13594
182. Song et al. (2026) Hippo — https://arxiv.org/abs/2610.24453
183. Li et al. (2026) BMAM — https://arxiv.org/abs/2601.20465
184. Li et al. (2026) HiMeS — https://arxiv.org/abs/2601.06152
185. Fang et al. (2025) Artificial Hippocampus Networks — https://arxiv.org/abs/2510.07318
186. He et al. (2024) CAMELoT — https://arxiv.org/abs/2402.13449
187. Zhang et al. (2024) Memory Mosaics — https://arxiv.org/abs/2405.06394
188. Sarthi et al. (2024) RAPTOR — https://arxiv.org/abs/2401.18059
189. Wang & Chen (2025) MIRIX — https://arxiv.org/abs/2507.07957
190. Hu, Wang & McAuley (2025) MemoryAgentBench — https://arxiv.org/abs/2507.05257
191. Pakhomov & Nijkamp (2026) Last-write resolver on MemoryAgentBench — https://arxiv.org/abs/2610.09193
192. Fan et al. (2026) VibeMemBench — https://arxiv.org/abs/2609.23570
193. Pink et al. (2025) Episodic memory position paper — https://arxiv.org/abs/2502.06975
194. Xu, Dai & Zhang (2026) "Contextual Agentic Memory is a Memo, Not True Memory" — https://arxiv.org/abs/2604.27707
195. Gu et al. (2026) FSFM — https://arxiv.org/abs/2604.20300
196. Jia et al. (2026) "The AI Hippocampus" survey — https://arxiv.org/abs/2601.09113
197. Park (2026) Position: Hippocampal explicit memory, ICML 2026 — https://arxiv.org/abs/2606.11245
