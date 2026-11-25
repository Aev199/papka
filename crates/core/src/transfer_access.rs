//! Read/write footprints for whole copy/move batches. Paths include directory trees.
use std::path::{Path,PathBuf};
#[derive(Clone,Debug)]
pub struct Access { pub path:PathBuf,pub write:bool }
impl Access {
    pub fn new(path:&Path,write:bool)->Self {
        let normalized=crate::path_utils::normalize_extended_path(path);
        let normalized=crate::path_utils::network_path(&normalized).unwrap_or(normalized);
        let path=if cfg!(windows){PathBuf::from(normalized.to_string_lossy().to_lowercase())}else{normalized};
        Self{path:path.components().collect(),write}
    }
}
pub fn conflicts(a:&[Access],b:&[Access])->bool {
    a.iter().any(|a|b.iter().any(|b|(a.write||b.write)
        &&(a.path.starts_with(&b.path)||b.path.starts_with(&a.path))))
}
pub fn transfer(sources:&[PathBuf],target:&Path,copy:bool,numbered:bool)->Vec<Access> {
    let mut accesses=Vec::new();
    for source in sources {
        accesses.push(Access::new(source,!copy));
        accesses.push(Access::new(&target.join(source.file_name().unwrap_or_default()),true));
    }
    // Numbered names are allocated by the executor; reserve their directory.
    if numbered {accesses.push(Access::new(target,true));}
    accesses
}
#[cfg(test)]
mod tests {
    use super::*;
    fn a(path:&str,write:bool)->Access {Access::new(Path::new(path),write)}
    #[test] fn independent_targets_and_shared_readers_can_run_together() {
        assert!(!conflicts(&[a("/src/a",false),a("/one/a",true)],&[a("/src/a",false),a("/two/a",true)]));
        assert!(!conflicts(&[a("/dst/a",true)],&[a("/dst/ab",true)]));
    }
    #[test] fn moves_destinations_and_ancestor_trees_are_exclusive() {
        assert!(conflicts(&[a("/src",true)],&[a("/src/a",false)]));
        assert!(conflicts(&[a("/dst/a",true)],&[a("/dst/a/child",true)]));
        let sources=vec![PathBuf::from("/src/a")];
        let numbered=transfer(&sources,Path::new("/dst"),true,true);
        assert!(conflicts(&numbered,&[a("/dst/a (2)",true)]));
    }
    #[cfg(windows)]
    #[test] fn windows_prefix_case_and_separators_do_not_bypass_locks() {
        assert!(conflicts(&[a(r"\\?\C:\Work\DIR",true)],&[a("c:/work/dir/file",false)]));
        assert!(conflicts(&[a(r"\\?\UNC\srv\share\Dir",true)],&[a(r"\\SRV\SHARE\dir\file",false)]));
    }
}
