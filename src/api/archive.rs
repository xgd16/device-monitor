//! 压缩/解压模块
//!
//! 支持 zip（纯 Rust）、7z（sevenz-rust）、rar（依赖系统 unrar/7z/rar 命令）。

use std::fs::{self, File};
use std::io::{copy, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    SevenZ,
    Rar,
}

impl ArchiveFormat {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "zip" => Ok(Self::Zip),
            "7z" => Ok(Self::SevenZ),
            "rar" => Ok(Self::Rar),
            _ => Err(format!("Unsupported format: {s}. Use zip, 7z, or rar")),
        }
    }

    pub fn from_path(path: &Path) -> Result<Self, String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .ok_or_else(|| "Cannot detect archive format from extension".to_string())?;
        Self::parse(&ext)
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZ => "7z",
            Self::Rar => "rar",
        }
    }
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent dir: {e}"))?;
        }
    }
    Ok(())
}

/// 防止 Zip Slip：解压目标必须在 dest 目录内。
fn safe_join(dest: &Path, name: &str) -> Result<PathBuf, String> {
    let dest = fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf());
    let joined = dest.join(name);
    let normalized = normalize_extract_path(&joined);
    if !normalized.starts_with(&dest) {
        return Err(format!("Unsafe path in archive: {name}"));
    }
    Ok(normalized)
}

fn normalize_extract_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        use std::path::Component;
        match comp {
            Component::RootDir | Component::Prefix(_) => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(p) => out.push(p),
        }
    }
    out
}

fn collect_files(base: &Path, root_name: &str, out: &mut Vec<(PathBuf, String)>) -> Result<(), String> {
    if base.is_file() {
        out.push((base.to_path_buf(), root_name.to_string()));
        return Ok(());
    }
    if !base.is_dir() {
        return Err(format!("Path is not a file or directory: {}", base.display()));
    }
    for entry in fs::read_dir(base).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let archive_name = if root_name.is_empty() {
            name.clone()
        } else {
            format!("{root_name}/{name}")
        };
        collect_files(&path, &archive_name, out)?;
    }
    Ok(())
}

pub fn compress(paths: &[PathBuf], output: &Path, format: ArchiveFormat) -> Result<u64, String> {
    if paths.is_empty() {
        return Err("No paths to compress".into());
    }
    ensure_parent(output)?;
    if output.exists() {
        return Err(format!("Output already exists: {}", output.display()));
    }

    tracing::info!(
        output = %output.display(),
        format = format.extension(),
        count = paths.len(),
        "archive: compress"
    );

    match format {
        ArchiveFormat::Zip => compress_zip(paths, output),
        ArchiveFormat::SevenZ => compress_7z(paths, output),
        ArchiveFormat::Rar => compress_rar(paths, output),
    }
}

fn compress_zip(paths: &[PathBuf], output: &Path) -> Result<u64, String> {
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let file = File::create(output).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut items: Vec<(PathBuf, String)> = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "item".into());
        collect_files(path, &name, &mut items)?;
    }

    for (fs_path, archive_name) in &items {
        zip.start_file(archive_name, options)
            .map_err(|e| format!("zip start_file: {e}"))?;
        let mut f = File::open(fs_path).map_err(|e| e.to_string())?;
        copy(&mut f, &mut zip).map_err(|e| e.to_string())?;
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(fs::metadata(output).map_err(|e| e.to_string())?.len())
}

fn compress_7z(paths: &[PathBuf], output: &Path) -> Result<u64, String> {
    use sevenz_rust::{SevenZArchiveEntry, SevenZWriter};

    let file = File::create(output).map_err(|e| e.to_string())?;
    let mut writer = SevenZWriter::new(file).map_err(|e| format!("7z writer: {e}"))?;

    for path in paths {
        let entry_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "item".into());

        if path.is_dir() {
            writer
                .push_source_path_non_solid(path, |_| true)
                .map_err(|e| format!("7z add dir: {e}"))?;
        } else {
            let entry = SevenZArchiveEntry::from_path(path, entry_name);
            let src = File::open(path).map_err(|e| e.to_string())?;
            writer
                .push_archive_entry(entry, Some(src))
                .map_err(|e| format!("7z add file: {e}"))?;
        }
    }

    writer.finish().map_err(|e| format!("7z finish: {e}"))?;
    Ok(fs::metadata(output).map_err(|e| e.to_string())?.len())
}

fn compress_rar(paths: &[PathBuf], output: &Path) -> Result<u64, String> {
    let rar_bin = find_command(&["rar", "/usr/bin/rar", "/system/bin/rar"])
        .ok_or_else(|| "RAR 压缩需要安装 rar 命令（RAR 为专有格式，无纯 Rust 实现）".to_string())?;

    let mut cmd = Command::new(&rar_bin);
    cmd.arg("a").arg("-ep1").arg("-idq").arg(output);
    for path in paths {
        cmd.arg(path);
    }

    let output_result = cmd.output().map_err(|e| format!("Failed to run rar: {e}"))?;
    if !output_result.status.success() {
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        return Err(format!("rar failed: {}", stderr.trim()));
    }
    Ok(fs::metadata(output).map_err(|e| e.to_string())?.len())
}

pub fn extract(archive: &Path, dest: &Path, overwrite: bool) -> Result<usize, String> {
    if !archive.is_file() {
        return Err("Archive path is not a file".into());
    }
    fs::create_dir_all(dest).map_err(|e| format!("Failed to create dest: {e}"))?;

    let format = ArchiveFormat::from_path(archive)?;
    tracing::info!(
        archive = %archive.display(),
        dest = %dest.display(),
        format = format.extension(),
        "archive: extract"
    );

    match format {
        ArchiveFormat::Zip => extract_zip(archive, dest, overwrite),
        ArchiveFormat::SevenZ => extract_7z(archive, dest, overwrite),
        ArchiveFormat::Rar => extract_rar(archive, dest, overwrite),
    }
}

fn extract_zip(archive: &Path, dest: &Path, overwrite: bool) -> Result<usize, String> {
    use zip::ZipArchive;

    let file = File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(BufReader::new(file)).map_err(|e| format!("Invalid zip: {e}"))?;
    let dest_canon = fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf());
    let mut count = 0usize;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().to_string();
        if name.ends_with('/') {
            let dir_path = safe_join(&dest_canon, &name)?;
            fs::create_dir_all(&dir_path).map_err(|e| e.to_string())?;
            continue;
        }
        let out_path = safe_join(&dest_canon, &name)?;
        if out_path.exists() && !overwrite {
            return Err(format!("File already exists: {}", out_path.display()));
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out_file = File::create(&out_path).map_err(|e| e.to_string())?;
        copy(&mut file, &mut out_file).map_err(|e| e.to_string())?;
        count += 1;
    }
    Ok(count)
}

fn extract_7z(archive: &Path, dest: &Path, overwrite: bool) -> Result<usize, String> {
    if !overwrite {
        let reader = sevenz_rust::SevenZReader::open(archive, sevenz_rust::Password::empty())
            .map_err(|e| format!("7z open: {e}"))?;
        let dest_canon = fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf());
        for entry in &reader.archive().files {
            if entry.is_directory() {
                continue;
            }
            let out = safe_join(&dest_canon, entry.name())?;
            if out.exists() {
                return Err(format!("File already exists: {}", out.display()));
            }
        }
    }
    sevenz_rust::decompress_file(archive, dest).map_err(|e| format!("7z extract: {e}"))?;
    count_files_recursive(dest)
}

fn extract_rar(archive: &Path, dest: &Path, overwrite: bool) -> Result<usize, String> {
    // 优先 unrar，其次 7z（7-Zip 可解压 RAR）
    if let Some(unrar) = find_command(&["unrar", "/usr/bin/unrar", "/system/bin/unrar"]) {
        let mut cmd = Command::new(&unrar);
        if overwrite {
            cmd.args(["x", "-o+", "-idq"]);
        } else {
            cmd.args(["x", "-o-", "-idq"]);
        }
        cmd.arg(archive).arg(dest);
        let out = cmd.output().map_err(|e| format!("Failed to run unrar: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("unrar failed: {}", stderr.trim()));
        }
        return count_files_recursive(dest);
    }

    if let Some(sevenz) = find_command(&["7z", "7zz", "/usr/bin/7z", "/system/bin/7z"]) {
        let mut cmd = Command::new(&sevenz);
        cmd.args(["x", "-y"]).arg(format!("-o{}", dest.display())).arg(archive);
        if !overwrite {
            // 7z 无简单 no-overwrite，用 -aos 跳过已存在
            cmd.arg("-aos");
        }
        let out = cmd.output().map_err(|e| format!("Failed to run 7z: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("7z extract failed: {}", stderr.trim()));
        }
        return count_files_recursive(dest);
    }

    Err("RAR 解压需要安装 unrar 或 7z 命令".into())
}

fn count_files_recursive(dir: &Path) -> Result<usize, String> {
    let mut count = 0usize;
    if !dir.is_dir() {
        return Ok(0);
    }
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            count += count_files_recursive(&path)?;
        } else {
            count += 1;
        }
    }
    Ok(count)
}

fn find_command(candidates: &[&str]) -> Option<String> {
    for name in candidates {
        if name.contains('/') {
            if Path::new(name).is_file() {
                return Some(name.to_string());
            }
        } else if Command::new(name)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            || Command::new(name).output().is_ok()
        {
            return Some(name.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_format() {
        assert_eq!(ArchiveFormat::parse("zip").unwrap(), ArchiveFormat::Zip);
        assert_eq!(ArchiveFormat::parse("7z").unwrap(), ArchiveFormat::SevenZ);
        assert_eq!(ArchiveFormat::parse("rar").unwrap(), ArchiveFormat::Rar);
    }
}
