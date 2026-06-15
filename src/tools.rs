pub struct ToolMeta {
    pub name: &'static str,
    pub write: bool,
    pub idempotent: bool,
    pub destructive: bool,
}

#[rustfmt::skip]
pub const ALL_TOOLS: &[ToolMeta] = &[
    ToolMeta { name: "read_text_file",              write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "read_media_file",             write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "write_file",                  write: true,  idempotent: true,  destructive: true  },
    ToolMeta { name: "edit_file",                   write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "create_directory",            write: true,  idempotent: true,  destructive: false },
    ToolMeta { name: "list_directory",              write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "list_directory_with_sizes",   write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "move_file",                   write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "copy_file",                   write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "delete_file",                 write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "delete_directory",            write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "search_files",                write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "directory_tree",              write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "get_file_info",               write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "list_allowed_directories",    write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "hash_file",                   write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "grep_files",                  write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "set_permissions",             write: true,  idempotent: true,  destructive: false },
    ToolMeta { name: "get_disk_usage",              write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "create_symlink",              write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "read_file_range",             write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "compress_gzip",               write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "decompress_gzip",             write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "compress_zstd",               write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "decompress_zstd",             write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "compress_tar",                write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "decompress_tar",              write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "encrypt_file",                write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "decrypt_file",                write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "generate_key",                write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_create",                  write: true,  idempotent: true,  destructive: false },
    ToolMeta { name: "csv_read",                    write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "csv_add_row",                 write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_update_cell",             write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_remove_row",              write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "csv_add_column",              write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_remove_column",           write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "csv_rename_column",           write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_read_column_values_range", write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "csv_read_row_range",           write: false, idempotent: true,  destructive: false },
];

#[inline]
pub fn tool_exists(name: &str) -> bool {
    ALL_TOOLS.iter().any(|t| t.name == name)
}

#[inline]
pub fn is_write_tool(name: &str) -> bool {
    ALL_TOOLS.iter().find(|t| t.name == name).map(|t| t.write).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_exists_known() {
        assert!(tool_exists("read_text_file"));
        assert!(tool_exists("write_file"));
        assert!(tool_exists("list_directory"));
    }

    #[test]
    fn test_tool_exists_unknown() {
        assert!(!tool_exists("nonexistent_tool"));
    }

    #[test]
    fn test_is_write_tool() {
        assert!(is_write_tool("write_file"));
        assert!(is_write_tool("delete_file"));
        assert!(!is_write_tool("read_text_file"));
        assert!(!is_write_tool("list_allowed_directories"));
    }

    #[test]
    fn test_all_tools_unique() {
        let mut names: Vec<&str> = ALL_TOOLS.iter().map(|t| t.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), ALL_TOOLS.len(), "Duplicate tool names");
    }
}
