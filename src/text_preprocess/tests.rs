use super::preprocess_for_llm;

#[test]
fn removes_numbers_symbols_and_stopwords() {
    let text = "Graph neural network 2024 improves accuracy by 12.5% on Cora.";
    let processed = preprocess_for_llm(text);

    assert!(processed.contains("graph neural network"));
    assert!(processed.contains("improves"));
    assert!(processed.contains("accuracy"));
    assert!(!processed.contains("2024"));
    assert!(!processed.contains('%'));
    assert!(!processed.contains(" by "));
}

#[test]
fn repairs_hyphenation_and_deduplicates_terms() {
    let text = "multi-\nmodal learning enables multi-\nmodal learning";
    let processed = preprocess_for_llm(text);

    assert!(processed.contains("multimodal learning enables"));
    assert_eq!(processed.matches("multimodal").count(), 1);
}

#[test]
fn removes_repeated_headers_and_page_numbers() {
    let text =
        "Conference 2024\n1\nGraph Neural Networks\nConference 2024\n2\nGraph Neural Networks";
    let processed = preprocess_for_llm(text);

    assert!(!processed.contains("conference"));
    assert!(!processed.contains("\n1"));
    assert!(processed.contains("graph neural networks"));
}

#[test]
fn drops_references_section() {
    let text = "Abstract\nGraph neural networks for molecules.\n\nReferences\n[1] Smith 2020";
    let processed = preprocess_for_llm(text);

    assert!(processed.contains("graph neural networks"));
    assert!(!processed.contains("smith"));
}

#[test]
fn discards_terms_with_two_or_fewer_characters() {
    let text = "AI for CV in 3D object detection and rl agents";
    let processed = preprocess_for_llm(text);

    assert!(processed.contains("object"));
    assert!(processed.contains("detection"));
    assert!(processed.contains("agents"));
    assert!(!processed.contains("ai"));
    assert!(!processed.contains("cv"));
    assert!(!processed.contains("rl"));
}
