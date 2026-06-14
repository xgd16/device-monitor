//! 文件管理 API
//!
//! 提供目录浏览、读写、上传下载、创建/删除/移动/复制等操作。
//! 路径经 canonicalize 规范化，防止 `../` 穿越攻击。

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Query};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};
use super::{error, success};

const DEFAULT_READ_LIMIT: usize = 256 * 1024;
const MAX_READ_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mode: String,
    pub owner: String,
    pub group: String,
    pub modified: i64,
    pub is_symlink: bool,
}

#[derive(Deserialize)]
pub struct PathQuery {
    pub path: Option<String>,
    /// 为 true 时使用 inline  disposition 并设置合适 Content-Type，供浏览器内嵌预览
    pub inline: Option<bool>,
}

#[derive(Deserialize)]
pub struct ReadQuery {
    pub path: String,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    pub path: String,
    pub recursive: Option<bool>,
}

#[derive(Deserialize)]
pub struct WriteBody {
    pub path: String,
    pub content: String,
    pub create: Option<bool>,
}

#[derive(Deserialize)]
pub struct PathBody {
    pub path: String,
}

#[derive(Deserialize)]
pub struct FromToBody {
    pub from: String,
    pub to: String,
}

#[derive(Deserialize)]
pub struct CompressBody {
    pub paths: Vec<String>,
    pub output: String,
    pub format: String,
}

#[derive(Deserialize)]
pub struct ExtractBody {
    pub path: String,
    pub dest: String,
    pub overwrite: Option<bool>,
}

/// 规范化已存在路径。
fn resolve_path(raw: &str) -> Result<PathBuf, String> {
    validate_raw_path(raw)?;
    let joined = normalize_path(raw);
    fs::canonicalize(&joined).map_err(|e| format!("Path not found: {}", e))
}

/// 规范化待创建路径（目标可能尚不存在）。
fn resolve_path_for_create(raw: &str) -> Result<PathBuf, String> {
    validate_raw_path(raw)?;
    let joined = normalize_path(raw);
    if joined.exists() {
        return fs::canonicalize(&joined).map_err(|e| e.to_string());
    }
    let file_name = joined
        .file_name()
        .ok_or_else(|| "Invalid path".to_string())?
        .to_owned();
    let parent = joined
        .parent()
        .ok_or_else(|| "Invalid path".to_string())?;
    let parent_canon = if parent.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        fs::canonicalize(parent).map_err(|e| format!("Parent path not found: {}", e))?
    };
    Ok(parent_canon.join(file_name))
}

fn validate_raw_path(raw: &str) -> Result<(), String> {
    if raw.is_empty() {
        return Err("Path is empty".into());
    }
    if raw.contains('\0') {
        return Err("Invalid path".into());
    }
    Ok(())
}

fn normalize_path(raw: &str) -> PathBuf {
    let path = if raw.starts_with('/') {
        PathBuf::from(raw)
    } else {
        PathBuf::from("/").join(raw)
    };
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::RootDir => out.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("/");
                }
            }
            Component::Normal(p) => out.push(p),
            Component::Prefix(_) => out.push(comp.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

fn mode_string(meta: &fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mode = meta.mode();
        format!(
            "{}{}{}{}{}{}{}{}{}",
            if mode & 0o400 != 0 { 'r' } else { '-' },
            if mode & 0o200 != 0 { 'w' } else { '-' },
            if mode & 0o100 != 0 { 'x' } else { '-' },
            if mode & 0o040 != 0 { 'r' } else { '-' },
            if mode & 0o020 != 0 { 'w' } else { '-' },
            if mode & 0o010 != 0 { 'x' } else { '-' },
            if mode & 0o004 != 0 { 'r' } else { '-' },
            if mode & 0o002 != 0 { 'w' } else { '-' },
            if mode & 0o001 != 0 { 'x' } else { '-' },
        )
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        "-".to_string()
    }
}

fn owner_group(_meta: &fs::Metadata) -> (String, String) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (format!("{}", _meta.uid()), format!("{}", _meta.gid()))
    }
    #[cfg(not(unix))]
    {
        ("-".into(), "-".into())
    }
}

fn entry_from_path(path: &Path) -> Result<FileEntry, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let is_symlink = meta.file_type().is_symlink();
    let (is_dir, size, modified) = if is_symlink {
        let target_meta = fs::metadata(path).map_err(|e| e.to_string())?;
        (
            target_meta.is_dir(),
            target_meta.len(),
            target_meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        )
    } else {
        (
            meta.is_dir(),
            meta.len(),
            meta.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        )
    };
    let (owner, group) = owner_group(&meta);
    Ok(FileEntry {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        path: path.display().to_string(),
        is_dir,
        size,
        mode: mode_string(&meta),
        owner,
        group,
        modified,
        is_symlink,
    })
}

fn parent_path(path: &Path) -> Option<String> {
    path.parent().map(|p| {
        if p.as_os_str().is_empty() || p == Path::new("/") {
            "/".to_string()
        } else {
            p.display().to_string()
        }
    })
}

/// `GET /api/files/list?path=`
pub async fn list_files(Query(q): Query<PathQuery>) -> Json<Value> {
    let raw = q.path.as_deref().unwrap_or("/");
    let path = match resolve_path(raw) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    if !path.is_dir() {
        return error("Not a directory");
    }

    tracing::info!(path = %path.display(), "files: list");

    let mut entries = Vec::new();
    let read_dir = match fs::read_dir(&path) {
        Ok(d) => d,
        Err(e) => return error(&format!("Failed to read directory: {}", e)),
    };

    for item in read_dir.flatten() {
        let entry_path = item.path();
        match entry_from_path(&entry_path) {
            Ok(entry) => entries.push(entry),
            Err(e) => tracing::warn!("Skip entry {}: {}", entry_path.display(), e),
        }
    }

    entries.sort_by(|a, b| {
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    success(serde_json::json!({
        "path": path.display().to_string(),
        "parent": parent_path(&path),
        "entries": entries,
    }))
}

/// `GET /api/files/stat?path=`
pub async fn stat_file(Query(q): Query<PathQuery>) -> Json<Value> {
    let raw = match q.path.as_deref() {
        Some(p) => p,
        None => return error("path is required"),
    };
    let path = match resolve_path(raw) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    match entry_from_path(&path) {
        Ok(entry) => success(entry),
        Err(e) => error(&e),
    }
}

/// `GET /api/files/read?path=&offset=&limit=`
pub async fn read_file(Query(q): Query<ReadQuery>) -> Json<Value> {
    let path = match resolve_path(&q.path) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    if path.is_dir() {
        return error("Cannot read a directory");
    }

    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(DEFAULT_READ_LIMIT).min(MAX_READ_LIMIT);

    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(e) => return error(&format!("Failed to read file: {}", e)),
    };

    let total = data.len();
    let slice = if offset >= total {
        &[]
    } else {
        &data[offset..total.min(offset + limit)]
    };

    let is_binary = slice.iter().take(8192).any(|&b| b == 0);
    let content = if is_binary {
        None
    } else {
        Some(String::from_utf8_lossy(slice).into_owned())
    };

    success(serde_json::json!({
        "path": path.display().to_string(),
        "offset": offset,
        "total_size": total,
        "read_size": slice.len(),
        "is_binary": is_binary,
        "content": content,
        "encoding": if is_binary { "binary" } else { "utf-8" },
    }))
}

/// `PUT /api/files/write`
pub async fn write_file(Json(body): Json<WriteBody>) -> Json<Value> {
    let path = match resolve_path_for_create(&body.path) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };

    if path.exists() && path.is_dir() {
        return error("Path is a directory");
    }
    if !path.exists() && !body.create.unwrap_or(false) {
        return error("File does not exist; set create=true to create");
    }

    tracing::info!(path = %path.display(), "files: write");

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            if let Err(e) = fs::create_dir_all(parent) {
                return error(&format!("Failed to create parent: {}", e));
            }
        }
    }

    match fs::write(&path, &body.content) {
        Ok(()) => success(serde_json::json!({
            "path": path.display().to_string(),
            "size": body.content.len(),
        })),
        Err(e) => error(&format!("Failed to write file: {}", e)),
    }
}

/// `POST /api/files/upload`
pub async fn upload_file(mut multipart: Multipart) -> Json<Value> {
    let mut target_dir: Option<String> = None;
    let mut uploaded: Vec<serde_json::Value> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name == "path" {
            if let Ok(text) = field.text().await {
                target_dir = Some(text);
            }
            continue;
        }
        if name == "file" {
            let dir_raw = match target_dir.as_deref() {
                Some(d) => d,
                None => return error("path field is required before file"),
            };
            let dir = match resolve_path(dir_raw) {
                Ok(p) => p,
                Err(e) => return error(&e),
            };
            if !dir.is_dir() {
                return error("Upload path is not a directory");
            }
            let file_name = field.file_name().unwrap_or("upload").to_string();
            let dest = dir.join(&file_name);
            let data = match field.bytes().await {
                Ok(b) => b,
                Err(e) => return error(&format!("Failed to read upload: {}", e)),
            };
            tracing::info!(path = %dest.display(), size = data.len(), "files: upload");
            if let Err(e) = fs::write(&dest, &data) {
                return error(&format!("Failed to save file: {}", e));
            }
            uploaded.push(serde_json::json!({
                "path": dest.display().to_string(),
                "size": data.len(),
            }));
        }
    }

    if uploaded.is_empty() {
        return error("No file uploaded");
    }
    success(serde_json::json!({ "uploaded": uploaded }))
}

/// `GET /api/files/download?path=&inline=`
pub async fn download_file(Query(q): Query<PathQuery>) -> Response {
    let raw = match q.path.as_deref() {
        Some(p) => p,
        None => {
            return (StatusCode::BAD_REQUEST, error("path is required")).into_response();
        }
    };
    let path = match resolve_path(raw) {
        Ok(p) => p,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, error(&e)).into_response();
        }
    };
    if path.is_dir() {
        return (StatusCode::BAD_REQUEST, error("Cannot download a directory")).into_response();
    }

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());

    let inline = q.inline.unwrap_or(false);
    let content_type = if inline {
        mime_type_for_path(&path)
    } else {
        "application/octet-stream"
    };
    let disposition = if inline {
        format!("inline; filename=\"{}\"", file_name)
    } else {
        format!("attachment; filename=\"{}\"", file_name)
    };

    match fs::read(&path) {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CONTENT_DISPOSITION, disposition)
            .body(Body::from(data))
            .unwrap(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            error(&format!("Failed to read file: {}", e)),
        )
            .into_response(),
    }
}

fn mime_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("avi") => "video/x-msvideo",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// `POST /api/files/mkdir`
pub async fn mkdir(Json(body): Json<PathBody>) -> Json<Value> {
    let path = match resolve_path_for_create(&body.path) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    tracing::info!(path = %path.display(), "files: mkdir");
    match fs::create_dir_all(&path) {
        Ok(()) => success(serde_json::json!({ "path": path.display().to_string() })),
        Err(e) => error(&format!("Failed to create directory: {}", e)),
    }
}

/// `POST /api/files/rename`
pub async fn rename_file(Json(body): Json<FromToBody>) -> Json<Value> {
    let from = match resolve_path(&body.from) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    let to = match resolve_path_for_create(&body.to) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    tracing::info!(from = %from.display(), to = %to.display(), "files: rename");
    match fs::rename(&from, &to) {
        Ok(()) => success(serde_json::json!({
            "from": from.display().to_string(),
            "to": to.display().to_string(),
        })),
        Err(e) => error(&format!("Failed to rename: {}", e)),
    }
}

/// `POST /api/files/move`
pub async fn move_file(Json(body): Json<FromToBody>) -> Json<Value> {
    rename_file(Json(body)).await
}

/// `POST /api/files/copy`
pub async fn copy_file(Json(body): Json<FromToBody>) -> Json<Value> {
    let from = match resolve_path(&body.from) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    let to = match resolve_path_for_create(&body.to) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    if from.is_dir() {
        return error("Directory copy not supported; use move instead");
    }
    tracing::info!(from = %from.display(), to = %to.display(), "files: copy");
    match fs::copy(&from, &to) {
        Ok(size) => success(serde_json::json!({
            "from": from.display().to_string(),
            "to": to.display().to_string(),
            "size": size,
        })),
        Err(e) => error(&format!("Failed to copy: {}", e)),
    }
}

/// `DELETE /api/files/delete?path=&recursive=`
pub async fn delete_file(Query(q): Query<DeleteQuery>) -> Json<Value> {
    let path = match resolve_path(&q.path) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };
    let recursive = q.recursive.unwrap_or(false);

    tracing::info!(path = %path.display(), recursive, "files: delete");

    let result = if path.is_dir() {
        if recursive {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_dir(&path)
        }
    } else {
        fs::remove_file(&path)
    };

    match result {
        Ok(()) => success(serde_json::json!({ "path": path.display().to_string() })),
        Err(e) => error(&format!("Failed to delete: {}", e)),
    }
}

/// `POST /api/files/compress`
pub async fn compress_files(Json(body): Json<CompressBody>) -> Json<Value> {
    if body.paths.is_empty() {
        return error("paths is required");
    }

    let format = match super::archive::ArchiveFormat::parse(&body.format) {
        Ok(f) => f,
        Err(e) => return error(&e),
    };

    let mut resolved_paths: Vec<PathBuf> = Vec::new();
    for raw in &body.paths {
        match resolve_path(raw) {
            Ok(p) => resolved_paths.push(p),
            Err(e) => return error(&e),
        }
    }

    let output = match resolve_path_for_create(&body.output) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };

    // 确保输出扩展名与格式一致
    let expected_ext = format.extension();
    if output.extension().and_then(|e| e.to_str()) != Some(expected_ext) {
        return error(&format!("Output file must have .{expected_ext} extension"));
    }

    match super::archive::compress(&resolved_paths, &output, format) {
        Ok(size) => success(serde_json::json!({
            "output": output.display().to_string(),
            "format": body.format,
            "size": size,
            "source_count": resolved_paths.len(),
        })),
        Err(e) => error(&e),
    }
}

/// `POST /api/files/extract`
pub async fn extract_files(Json(body): Json<ExtractBody>) -> Json<Value> {
    let archive = match resolve_path(&body.path) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };

    let dest = match resolve_path_for_create(&body.dest) {
        Ok(p) => p,
        Err(e) => return error(&e),
    };

    let overwrite = body.overwrite.unwrap_or(false);

    match super::archive::extract(&archive, &dest, overwrite) {
        Ok(count) => success(serde_json::json!({
            "archive": archive.display().to_string(),
            "dest": dest.display().to_string(),
            "extracted_files": count,
        })),
        Err(e) => error(&e),
    }
}
