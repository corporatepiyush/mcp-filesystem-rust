//! Single source of truth for all MCP tools and their categories.
//!
//! Tools are grouped into [`ToolCategory`] banners. **No tool is exposed by
//! default** — each category must be explicitly enabled at startup with the
//! matching `--enable-<slug>` flag (or `--enable-all`). A tool whose category is
//! disabled is hidden from `tools/list` and rejected from `tools/call` as if it
//! did not exist.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Coarse capability groups used to selectively expose tools at startup.
/// Maps one-to-one to a `--enable-<slug>` flag; keep at or below ten variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolCategory {
    /// Read-only inspection: read files, list/search/stat, hashes, disk usage.
    Read,
    /// Mutating filesystem ops: write/edit, create dir, move/copy, perms, symlink.
    Write,
    /// Destructive removal: delete file/directory.
    Delete,
    /// Compression & archives: gzip, zstd, tar.
    Compress,
    /// Encryption: encrypt/decrypt files and key generation.
    Crypto,
    /// CSV read/write helpers.
    Csv,
}

impl ToolCategory {
    pub const ALL: &'static [ToolCategory] = &[
        ToolCategory::Read,
        ToolCategory::Write,
        ToolCategory::Delete,
        ToolCategory::Compress,
        ToolCategory::Crypto,
        ToolCategory::Csv,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            ToolCategory::Read => "read",
            ToolCategory::Write => "write",
            ToolCategory::Delete => "delete",
            ToolCategory::Compress => "compress",
            ToolCategory::Crypto => "crypto",
            ToolCategory::Csv => "csv",
        }
    }
}

impl fmt::Display for ToolCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

impl FromStr for ToolCategory {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_lowercase().replace('_', "-").as_str() {
            "read" => Ok(ToolCategory::Read),
            "write" => Ok(ToolCategory::Write),
            "delete" => Ok(ToolCategory::Delete),
            "compress" => Ok(ToolCategory::Compress),
            "crypto" => Ok(ToolCategory::Crypto),
            "csv" => Ok(ToolCategory::Csv),
            _ => Err(format!("Unknown tool category: {s}")),
        }
    }
}

pub struct ToolMeta {
    pub name: &'static str,
    pub category: ToolCategory,
    pub write: bool,
    pub idempotent: bool,
    pub destructive: bool,
}

use ToolCategory::{Compress, Crypto, Csv, Delete, Read, Write};

#[rustfmt::skip]
pub const ALL_TOOLS: &[ToolMeta] = &[
    ToolMeta { name: "read_text_file",              category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "read_media_file",             category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "write_file",                  category: Write,    write: true,  idempotent: true,  destructive: true  },
    ToolMeta { name: "edit_file",                   category: Write,    write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "create_directory",            category: Write,    write: true,  idempotent: true,  destructive: false },
    ToolMeta { name: "list_directory",              category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "list_directory_with_sizes",   category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "move_file",                   category: Write,    write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "copy_file",                   category: Write,    write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "delete_file",                 category: Delete,   write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "delete_directory",            category: Delete,   write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "search_files",                category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "directory_tree",              category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "get_file_info",               category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "list_allowed_directories",    category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "hash_file",                   category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "grep_files",                  category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "set_permissions",             category: Write,    write: true,  idempotent: true,  destructive: false },
    ToolMeta { name: "get_disk_usage",              category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "create_symlink",              category: Write,    write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "read_file_range",             category: Read,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "compress_gzip",               category: Compress, write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "decompress_gzip",             category: Compress, write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "compress_zstd",               category: Compress, write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "decompress_zstd",             category: Compress, write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "compress_tar",                category: Compress, write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "decompress_tar",              category: Compress, write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "encrypt_file",                category: Crypto,   write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "decrypt_file",                category: Crypto,   write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "generate_key",                category: Crypto,   write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_create",                  category: Csv,      write: true,  idempotent: true,  destructive: false },
    ToolMeta { name: "csv_read",                    category: Csv,      write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "csv_add_row",                 category: Csv,      write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_update_cell",             category: Csv,      write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_remove_row",              category: Csv,      write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "csv_add_column",              category: Csv,      write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_remove_column",           category: Csv,      write: true,  idempotent: false, destructive: true  },
    ToolMeta { name: "csv_rename_column",           category: Csv,      write: true,  idempotent: false, destructive: false },
    ToolMeta { name: "csv_read_column_values_range", category: Csv,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "csv_read_row_range",           category: Csv,     write: false, idempotent: true,  destructive: false },
    ToolMeta { name: "csv_select_column_range",      category: Csv,     write: false, idempotent: true,  destructive: false },
];

#[inline]
fn lookup(name: &str) -> Option<&'static ToolMeta> {
    ALL_TOOLS.iter().find(|t| t.name == name)
}

#[inline]
pub fn tool_exists(name: &str) -> bool {
    lookup(name).is_some()
}

#[inline]
pub fn is_write_tool(name: &str) -> bool {
    lookup(name).map(|t| t.write).unwrap_or(false)
}

/// The category a tool belongs to, or `None` if the tool is unknown.
#[inline]
pub fn category_of(name: &str) -> Option<ToolCategory> {
    lookup(name).map(|t| t.category)
}

/// Whether a tool is callable given the set of enabled categories. A tool is
/// available only if it exists *and* its category is enabled.
#[inline]
pub fn is_tool_available(name: &str, enabled: &[ToolCategory]) -> bool {
    category_of(name).is_some_and(|c| enabled.contains(&c))
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

    #[test]
    fn test_every_tool_has_category() {
        for meta in ALL_TOOLS {
            assert_eq!(category_of(meta.name), Some(meta.category));
        }
        assert_eq!(category_of("nonexistent_tool"), None);
    }

    #[test]
    fn test_is_tool_available_gating() {
        assert!(!is_tool_available("read_text_file", &[]));
        assert!(is_tool_available("read_text_file", &[ToolCategory::Read]));
        assert!(!is_tool_available("read_text_file", &[ToolCategory::Write]));
        assert!(!is_tool_available("nonexistent_tool", ToolCategory::ALL));
    }

    #[test]
    fn test_category_slug_roundtrip() {
        for &cat in ToolCategory::ALL {
            assert_eq!(cat.slug().parse::<ToolCategory>().unwrap(), cat);
        }
        assert!("bogus".parse::<ToolCategory>().is_err());
    }

    #[test]
    fn test_categories_within_limit() {
        assert!(ToolCategory::ALL.len() <= 10);
    }
}
