use crate::api::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDir {
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDirEntry {
    pub name: String,
    pub is_dir: bool,
    pub metadata: Metadata,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub size: u64,
    pub created_at: Option<u64>,
    pub modified_at: Option<u64>,
}

pub async fn list_dir(Json(payload): Json<ListDir>) -> ApiResult {
    let mut entries = Vec::new();
    let path = std::path::Path::new(&payload.path);
    if path.is_dir() {
        let mut read_dir = tokio::fs::read_dir(path).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let metadata = entry.metadata().await?;
            entries.push(ListDirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: metadata.is_dir(),
                metadata: Metadata {
                    size: metadata.len(),
                    created_at: metadata
                        .created()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs()),
                    modified_at: metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs()),
                },
            });
        }
    } else {
        return Err(ApiError::InvalidInput(
            "Path is not a directory".to_string(),
        ));
    }
    ApiResponse::ok(entries)
}
