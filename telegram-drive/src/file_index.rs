use crate::database::Database;
use crate::models::{AppResult, Folder, FolderListResponse, SearchQuery};

#[derive(Clone)]
pub struct FileIndexService {
    db: Database,
}

impl FileIndexService {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn list_folder(
        &self,
        folder_id: i64,
        page: u32,
        page_size: u32,
    ) -> AppResult<FolderListResponse> {
        self.db.list_folder(folder_id, page, page_size)
    }

    pub fn search(&self, query: SearchQuery) -> AppResult<FolderListResponse> {
        self.db.search(query)
    }

    pub fn create_folder(&self, parent_id: Option<i64>, name: String) -> AppResult<Folder> {
        self.db.create_folder(parent_id, &name)
    }

    pub fn list_tree(&self) -> AppResult<Vec<Folder>> {
        self.db.list_all_folders()
    }
}
