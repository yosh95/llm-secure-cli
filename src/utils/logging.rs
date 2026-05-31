use std::fs;
use std::path::Path;

pub fn rotate_file(path: &Path, max_archives: usize) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid path"))?;

    // Shift existing archives: .1 -> .2, .2 -> .3, etc.
    for i in (1..max_archives).rev() {
        let mut old_name = file_name.to_os_string();
        old_name.push(format!(".{i}"));
        let old_archive = path.with_file_name(old_name);

        if old_archive.exists() {
            let mut new_name = file_name.to_os_string();
            new_name.push(format!(".{}", i + 1));
            let new_archive = path.with_file_name(new_name);
            fs::rename(old_archive, new_archive)?;
        }
    }

    // Move current file to .1
    let mut archive_name = file_name.to_os_string();
    archive_name.push(".1");
    let archive_path = path.with_file_name(archive_name);
    fs::rename(path, archive_path)?;

    Ok(())
}
