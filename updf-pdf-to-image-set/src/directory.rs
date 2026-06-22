// Contains items pertaining to directories

use std::{error::Error, path::PathBuf};

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

  pub fn create_dir_cwd( new_dir_name: String )
  -> Result<(), Box<dyn Error + 'static>>  {
    std::fs::create_dir(new_dir_name)?;
    Ok(())
  }

  pub fn create_all_dirs(base_dir: String, new_dir_chain: &str)
    -> Result<(), Box<dyn Error + 'static>> {
    let full_path = String::from(base_dir + new_dir_chain);

    std::fs::create_dir_all(full_path)?;
    Ok(())
  }


}
