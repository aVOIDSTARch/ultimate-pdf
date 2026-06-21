// Contains items pertaining to directories

use std::{fs, path::PathBuf};

pub struct PermissionsSet {
  readable: char,
  writable: char,
  executable: char
}

pub struct Directory {
  path: PathBuf,
  permissions: PermissionsSet
}

impl Directory {

  pub fn create_dir_cwd( new_dir_name: String )-> () {
    std::fs::create_dir(new_dir_name);
  }

  pub fn create_all_dirs(base_dir: String, new_dir_chain: &str) -> () {
    let full_path = String::from({base_dir + new_dir_chain});

    std::fs::create_dir_all(full_path);
  }


}
