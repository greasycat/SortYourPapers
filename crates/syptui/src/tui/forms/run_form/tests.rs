use std::fs;

use tempfile::tempdir;

use super::list_relative_directories;

#[test]
fn list_relative_directories_lists_children_for_existing_relative_dir() {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join("papers/nlp")).expect("create nlp dir");
    fs::create_dir_all(temp.path().join("papers/ml")).expect("create ml dir");

    let directories = list_relative_directories(temp.path(), "papers");

    assert_eq!(directories, vec!["papers/ml", "papers/nlp"]);
}

#[test]
fn list_relative_directories_filters_partial_relative_path() {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join("papers")).expect("create papers dir");
    fs::create_dir_all(temp.path().join("reports")).expect("create reports dir");

    let directories = list_relative_directories(temp.path(), "pa");

    assert_eq!(directories, vec!["papers"]);
}
