use filemanager_core::{browser::scan_directory,selection::visible_indices,sort::SortSpec};

#[cfg(windows)]
fn mark_hidden(path:&std::path::Path) {
    use std::os::windows::{ffi::OsStrExt,fs::MetadataExt};
    let flags=std::fs::metadata(path).unwrap().file_attributes()|2;
    let wide:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();
    assert_ne!(unsafe{windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(wide.as_ptr(),flags)},0);
}

#[test]
fn hidden_files_and_folders_are_both_controlled_by_visibility_preference() {
    let dir=tempfile::tempdir().unwrap();
    let file=dir.path().join(".hidden-file");let folder=dir.path().join(".hidden-folder");
    std::fs::write(&file,b"hidden").unwrap();std::fs::create_dir(&folder).unwrap();
    std::fs::write(dir.path().join("visible.txt"),b"visible").unwrap();
    #[cfg(windows)] {mark_hidden(&file);mark_hidden(&folder);}
    let listing=scan_directory(dir.path(),100).unwrap();
    let hidden:Vec<_>=listing.entries.iter().filter(|e|e.hidden).collect();
    assert_eq!(hidden.len(),2);
    assert!(hidden.iter().any(|e|e.is_directory));assert!(hidden.iter().any(|e|!e.is_directory));
    let normal=visible_indices(&listing.entries,SortSpec::default(),false,"");
    assert_eq!(normal.len(),1);assert_eq!(listing.entries[normal[0]].name,"visible.txt");
    assert_eq!(visible_indices(&listing.entries,SortSpec::default(),true,"").len(),3);
}
