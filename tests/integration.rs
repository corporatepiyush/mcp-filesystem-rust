use mcp_filesystem::protocol::JsonRpcRequest;
use mcp_filesystem::server::process_request;
use serde_json::{json};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::fs;

static TEST_DIR: OnceLock<PathBuf> = OnceLock::new();

fn test_dir() -> &'static PathBuf {
    TEST_DIR.get_or_init(|| {
        // Use the project's target directory to avoid macOS /tmp or /var symlinks
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test_tmp")
            .join(format!("mcp_fs_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    })
}

fn test_config() -> mcp_filesystem::config::Config {
    let dir = test_dir().to_string_lossy().to_string();
    mcp_filesystem::config::Config {
        allowed_directories: vec![dir],
        server: mcp_filesystem::config::ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
            http_port: 0,
            request_timeout: std::time::Duration::from_secs(5),
            access_mode: mcp_filesystem::config::AccessMode::Unrestricted,
            follow_symlinks: false,
        },
        max_file_size: 100 * 1024 * 1024,
    }
}

fn t(path: &str) -> PathBuf {
    test_dir().join(path)
}

// ────────────────────────────
//  File Tools
// ────────────────────────────

#[tokio::test]
async fn test_write_and_read_file() {
    let config = test_config();
    let path = t("hello.txt");

    let args = json!({ "path": &path, "content": "Hello, World!" });
    let res = mcp_filesystem::actions::files::write_file(Some(&args), &config).await;
    assert!(res.is_ok(), "write_file failed: {:?}", res.err());

    let args = json!({ "path": &path });
    let res = mcp_filesystem::actions::files::read_text_file(Some(&args), &config).await;
    assert!(res.is_ok(), "read_text_file failed: {:?}", res.err());
    let val = res.unwrap();
    assert_eq!(val["content"], "Hello, World!");
    assert_eq!(val["size"], 13);
}

#[tokio::test]
async fn test_read_file_head_tail() {
    let config = test_config();
    let path = t("lines.txt");
    let content = (1..=100).map(|i| format!("Line {i}")).collect::<Vec<_>>().join("\n");
    fs::write(&path, &content).unwrap();

    // Head 3
    let args = json!({ "path": &path, "head": 3 });
    let res = mcp_filesystem::actions::files::read_text_file(Some(&args), &config).await;
    assert!(res.is_ok());
    let val = res.unwrap();
    assert_eq!(val["content"], "Line 1\nLine 2\nLine 3");
    assert_eq!(val["totalLines"], 3);

    // Tail 2
    let args = json!({ "path": &path, "tail": 2 });
    let res = mcp_filesystem::actions::files::read_text_file(Some(&args), &config).await;
    assert!(res.is_ok());
    let val = res.unwrap();
    assert_eq!(val["content"], "Line 99\nLine 100");

    // Head and tail together should error
    let args = json!({ "path": &path, "head": 3, "tail": 2 });
    let res = mcp_filesystem::actions::files::read_text_file(Some(&args), &config).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_edit_file() {
    let config = test_config();
    let path = t("edit.txt");
    fs::write(&path, "Hello World").unwrap();

    let edits = json!([{"oldText": "World", "newText": "Rust"}]);
    let args = json!({ "path": &path, "edits": edits });
    let res = mcp_filesystem::actions::files::edit_file(Some(&args), &config).await;
    assert!(res.is_ok(), "edit_file failed: {:?}", res.err());

    let content = fs::read_to_string(&path).unwrap();
    assert_eq!(content, "Hello Rust");
}

#[tokio::test]
async fn test_move_file() {
    let config = test_config();
    fs::write(t("move_src.txt"), "move me").unwrap();
    let args = json!({ "source": t("move_src.txt"), "destination": t("move_dst.txt") });
    let res = mcp_filesystem::actions::files::move_file(Some(&args), &config).await;
    assert!(res.is_ok(), "move_file failed: {:?}", res.err());
    assert!(!t("move_src.txt").exists());
    assert!(t("move_dst.txt").exists());
}

#[tokio::test]
async fn test_copy_file() {
    let config = test_config();
    fs::write(t("copy_src.txt"), "copy me").unwrap();
    let args = json!({ "source": t("copy_src.txt"), "destination": t("copy_dst.txt") });
    let res = mcp_filesystem::actions::files::copy_file(Some(&args), &config).await;
    assert!(res.is_ok(), "copy_file failed: {:?}", res.err());
    assert!(t("copy_src.txt").exists());
    assert!(t("copy_dst.txt").exists());
}

#[tokio::test]
async fn test_delete_file() {
    let config = test_config();
    fs::write(t("delete_me.txt"), "bye").unwrap();
    let args = json!({ "path": t("delete_me.txt") });
    let res = mcp_filesystem::actions::files::delete_file(Some(&args), &config).await;
    assert!(res.is_ok(), "delete_file failed: {:?}", res.err());
    assert!(!t("delete_me.txt").exists());
}

#[tokio::test]
async fn test_get_file_info() {
    let config = test_config();
    fs::write(t("info.txt"), "info").unwrap();
    let args = json!({ "path": t("info.txt") });
    let res = mcp_filesystem::actions::files::get_file_info(Some(&args), &config).await;
    assert!(res.is_ok(), "get_file_info failed: {:?}", res.err());
    let val = res.unwrap();
    assert_eq!(val["type"], "file");
    assert!(val["size"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_delete_directory() {
    let config = test_config();
    fs::create_dir_all(t("del_dir/sub")).unwrap();
    fs::write(t("del_dir/sub/f.txt"), "x").unwrap();
    let args = json!({ "path": t("del_dir"), "recursive": true });
    let res = mcp_filesystem::actions::files::delete_directory(Some(&args), &config).await;
    assert!(res.is_ok(), "delete_directory failed: {:?}", res.err());
    assert!(!t("del_dir").exists());
}

// ────────────────────────────
//  Security / Validation
// ────────────────────────────

#[tokio::test]
async fn test_path_traversal_rejected() {
    let config = test_config();
    let args = json!({ "path": "../etc/passwd" });
    let res = mcp_filesystem::actions::files::read_text_file(Some(&args), &config).await;
    assert!(res.is_err(), "Path traversal should be rejected");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("not allowed") || err.contains("not found") || err.contains("not in allowed"), "Unexpected error: {err}");
}

#[tokio::test]
async fn test_write_to_traversal_rejected() {
    let config = test_config();
    let args = json!({ "path": "../escape.txt", "content": "bad" });
    let res = mcp_filesystem::actions::files::write_file(Some(&args), &config).await;
    assert!(res.is_err(), "Write outside allowed dirs should be rejected");
}

#[tokio::test]
async fn test_destination_outside_allowed_rejected() {
    let config = test_config();
    let args = json!({ "source": t("nonexistent"), "destination": "/etc/evil.txt" });
    let res = mcp_filesystem::actions::files::move_file(Some(&args), &config).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_symlink_component_rejected() {
    let config = test_config();
    // Create a symlink inside the allowed dir pointing outside
    let link = t("escape_link");
    let _ = fs::remove_file(&link);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/tmp", &link).unwrap();
        // Walking into the symlink should fail (follow_symlinks=false)
        let args = json!({ "path": t("escape_link") });
        let res = mcp_filesystem::actions::files::list_directory(Some(&args), &config).await;
        assert!(res.is_err(), "Symlink to outside should be rejected when follow_symlinks=false");
        let _ = fs::remove_file(&link);
    }
}

// ────────────────────────────
//  Search & Grep
// ────────────────────────────

#[tokio::test]
async fn test_search_files() {
    let config = test_config();
    let dir = t("search_test");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("data.csv"), "a,b,c").unwrap();
    fs::write(dir.join("data.json"), "{}").unwrap();

    let args = json!({ "path": &dir, "pattern": "*.csv" });
    let res = mcp_filesystem::actions::files::search_files(Some(&args), &config).await;
    assert!(res.is_ok(), "search_files failed: {:?}", res.err());
    let val = res.unwrap();
    assert_eq!(val["count"], 1, "Should find 1 csv file in search_test");
    assert!(val["results"][0].as_str().unwrap().ends_with("data.csv"));
}

#[tokio::test]
async fn test_grep_files() {
    let config = test_config();
    let dir = t("grep_test_dir");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("grep_test.txt"), "apple\nbanana\napple pie\n").unwrap();

    let args = json!({ "path": &dir, "pattern": "apple" });
    let res = mcp_filesystem::actions::files::grep_files(Some(&args), &config).await;
    assert!(res.is_ok(), "grep_files failed: {:?}", res.err());
    let val = res.unwrap();
    assert_eq!(val["count"], 2, "Should find 2 lines with 'apple'");
}

// ────────────────────────────
//  Compression
// ────────────────────────────

#[tokio::test]
async fn test_gzip_roundtrip() {
    let config = test_config();
    fs::write(t("gz_orig.txt"), "gzip compress me! ".repeat(1000)).unwrap();

    let args = json!({ "path": t("gz_orig.txt") });
    let res = mcp_filesystem::actions::files::compress_gzip(Some(&args), &config).await;
    assert!(res.is_ok(), "gzip compress failed: {:?}", res.err());

    let args = json!({ "path": t("gz_orig.txt.gz") });
    let res = mcp_filesystem::actions::files::decompress_gzip(Some(&args), &config).await;
    assert!(res.is_ok(), "gzip decompress failed: {:?}", res.err());

    let content = fs::read_to_string(t("gz_orig.txt")).unwrap();
    assert!(content.starts_with("gzip compress me!"));
}

#[tokio::test]
async fn test_zstd_roundtrip() {
    let config = test_config();
    fs::write(t("zst_orig.txt"), "zstd compress me! ".repeat(1000)).unwrap();

    let args = json!({ "path": t("zst_orig.txt") });
    let res = mcp_filesystem::actions::files::compress_zstd(Some(&args), &config).await;
    assert!(res.is_ok(), "zstd compress failed: {:?}", res.err());

    let args = json!({ "path": t("zst_orig.txt.zst") });
    let res = mcp_filesystem::actions::files::decompress_zstd(Some(&args), &config).await;
    assert!(res.is_ok(), "zstd decompress failed: {:?}", res.err());

    let content = fs::read_to_string(t("zst_orig.txt")).unwrap();
    assert!(content.starts_with("zstd compress me!"));
}

#[tokio::test]
async fn test_tar_roundtrip() {
    let config = test_config();
    let tar_dir = t("tar_dir");
    let archive = tar_dir.join("archive.tar");
    let extracted = tar_dir.join("extracted");
    fs::create_dir_all(&tar_dir).unwrap();
    fs::create_dir_all(&extracted).unwrap();
    fs::write(tar_dir.join("a.txt"), "hello").unwrap();
    fs::write(tar_dir.join("b.txt"), "world").unwrap();

    let args = json!({ "source": &tar_dir, "output": &archive });
    let res = mcp_filesystem::actions::files::compress_tar(Some(&args), &config).await;
    assert!(res.is_ok(), "tar compress failed: {:?}", res.err());

    let args = json!({ "path": &archive, "outputDir": &extracted });
    let res = mcp_filesystem::actions::files::decompress_tar(Some(&args), &config).await;
    assert!(res.is_ok(), "tar decompress failed: {:?}", res.err());
    assert!(extracted.join("a.txt").exists(), "a.txt should be extracted");
    assert!(extracted.join("b.txt").exists(), "b.txt should be extracted");
}

// ────────────────────────────
//  Crypto
// ────────────────────────────

#[tokio::test]
async fn test_aes_encrypt_decrypt_roundtrip() {
    let config = test_config();
    fs::write(t("secret.txt"), "This is secret data!").unwrap();

    let args = json!({
        "path": t("secret.txt"),
        "algorithm": "aes-256-gcm",
        "generateKey": true,
    });
    let res = mcp_filesystem::actions::crypto::encrypt_file(Some(&args), &config).await;
    assert!(res.is_ok(), "AES encrypt_file failed: {:?}", res.err());
    let val = res.unwrap();
    let key = val["key"].as_str().unwrap().to_string();
    assert!(t("secret.txt.enc").exists());

    let args = json!({
        "path": t("secret.txt.enc"),
        "key": &key,
    });
    let res = mcp_filesystem::actions::crypto::decrypt_file(Some(&args), &config).await;
    assert!(res.is_ok(), "AES decrypt_file failed: {:?}", res.err());

    let decrypted = fs::read_to_string(t("secret.txt")).unwrap();
    assert_eq!(decrypted, "This is secret data!");
}

#[tokio::test]
async fn test_chacha20_encrypt_decrypt_roundtrip() {
    let config = test_config();
    fs::write(t("chacha_secret.txt"), "ChaCha secret!").unwrap();

    let args = json!({
        "path": t("chacha_secret.txt"),
        "algorithm": "chacha20-poly1305",
        "generateKey": true,
    });
    let res = mcp_filesystem::actions::crypto::encrypt_file(Some(&args), &config).await;
    assert!(res.is_ok(), "ChaCha20 encrypt_file failed: {:?}", res.err());
    let val = res.unwrap();
    let key = val["key"].as_str().unwrap().to_string();

    let args = json!({
        "path": t("chacha_secret.txt.enc"),
        "key": &key,
    });
    let res = mcp_filesystem::actions::crypto::decrypt_file(Some(&args), &config).await;
    assert!(res.is_ok(), "ChaCha20 decrypt_file failed: {:?}", res.err());

    let decrypted = fs::read_to_string(t("chacha_secret.txt")).unwrap();
    assert_eq!(decrypted, "ChaCha secret!");
}

#[tokio::test]
async fn test_rsa_encrypt_decrypt_roundtrip() {
    let config = test_config();
    fs::write(t("rsa_secret.txt"), "RSA secret!").unwrap();

    // Generate RSA key
    let args = json!({ "algorithm": "rsa-2048" });
    let res = mcp_filesystem::actions::crypto::generate_key(Some(&args), &config).await;
    assert!(res.is_ok(), "RSA keygen failed: {:?}", res.err());
    let val = res.unwrap();
    let pub_key = val["publicKey"].as_str().unwrap().to_string();
    let priv_key = val["privateKey"].as_str().unwrap().to_string();

    // Encrypt with public key
    let args = json!({
        "path": t("rsa_secret.txt"),
        "algorithm": "rsa-2048-oaep",
        "publicKey": &pub_key,
    });
    let res = mcp_filesystem::actions::crypto::encrypt_file(Some(&args), &config).await;
    assert!(res.is_ok(), "RSA encrypt_file failed: {:?}", res.err());

    // Decrypt with private key
    let args = json!({
        "path": t("rsa_secret.txt.enc"),
        "privateKey": &priv_key,
    });
    let res = mcp_filesystem::actions::crypto::decrypt_file(Some(&args), &config).await;
    assert!(res.is_ok(), "RSA decrypt_file failed: {:?}", res.err());

    let decrypted = fs::read_to_string(t("rsa_secret.txt")).unwrap();
    assert_eq!(decrypted, "RSA secret!");
}

#[tokio::test]
async fn test_encrypt_wrong_key_fails() {
    let config = test_config();
    fs::write(t("wrong_key.txt"), "secret").unwrap();

    let args = json!({
        "path": t("wrong_key.txt"),
        "algorithm": "aes-256-gcm",
        "generateKey": true,
    });
    let res = mcp_filesystem::actions::crypto::encrypt_file(Some(&args), &config).await;
    assert!(res.is_ok());
    let val = res.unwrap();
    let _key = val["key"].as_str().unwrap().to_string();

    // Decrypt with wrong key
    let wrong_key = "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbe";
    let args = json!({
        "path": t("wrong_key.txt.enc"),
        "key": wrong_key,
    });
    let res = mcp_filesystem::actions::crypto::decrypt_file(Some(&args), &config).await;
    assert!(res.is_err(), "Decrypt with wrong key should fail");
}

// ────────────────────────────
//  CSV Tools
// ────────────────────────────

#[tokio::test]
async fn test_csv_create_and_read() {
    let config = test_config();
    let path = t("test.csv");

    let args = json!({
        "path": &path,
        "headers": ["Name", "Age", "City"],
        "rows": [["Alice", "30", "NYC"], ["Bob", "25", "SF"]],
    });
    let res = mcp_filesystem::actions::csv::csv_create(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_create failed: {:?}", res.err());
    assert!(path.exists());

    let args = json!({ "path": &path });
    let res = mcp_filesystem::actions::csv::csv_read(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_read failed: {:?}", res.err());
    let val = res.unwrap();
    assert_eq!(val["headers"].as_array().unwrap().len(), 3);
    assert_eq!(val["totalRows"], 2);
    assert_eq!(val["rows"][0][0], "Alice");
}

#[tokio::test]
async fn test_csv_read_pagination() {
    let config = test_config();
    let path = t("pagination.csv");
    let rows: Vec<Vec<String>> = (0..20).map(|i| vec![format!("val{i}")]).collect();
    let args = json!({
        "path": &path,
        "headers": ["col"],
        "rows": rows,
    });
    mcp_filesystem::actions::csv::csv_create(Some(&args), &config).await.unwrap();

    // Read with limit + offset
    let args = json!({ "path": &path, "limit": 5, "offset": 3 });
    let res = mcp_filesystem::actions::csv::csv_read(Some(&args), &config).await.unwrap();
    assert_eq!(res["totalRows"], 20);
    assert_eq!(res["returnedRows"], 5);
    assert_eq!(res["rows"][0][0], "val3");
}

#[tokio::test]
async fn test_csv_add_row() {
    let config = test_config();
    let path = t("addrow.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["X", "Y"],
    })), &config).await.unwrap();

    // Add rows via arrays
    let args = json!({
        "path": &path,
        "rows": [["10", "20"], ["30", "40"]],
    });
    let res = mcp_filesystem::actions::csv::csv_add_row(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_add_row failed: {:?}", res.err());
    assert_eq!(res.unwrap()["rowsAdded"], 2);

    // Add rows via objects
    let args = json!({
        "path": &path,
        "rows": [{"X": "50", "Y": "60"}],
    });
    let res = mcp_filesystem::actions::csv::csv_add_row(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_add_row objects failed: {:?}", res.err());

    let args = json!({ "path": &path });
    let val = mcp_filesystem::actions::csv::csv_read(Some(&args), &config).await.unwrap();
    assert_eq!(val["totalRows"], 3);
    assert_eq!(val["rows"][2][0], "50");
}

#[tokio::test]
async fn test_csv_update_cell() {
    let config = test_config();
    let path = t("update.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["A", "B"],
        "rows": [["old", "value"]],
    })), &config).await.unwrap();

    let args = json!({
        "path": &path, "row": 0, "column": "A", "value": "new",
    });
    let res = mcp_filesystem::actions::csv::csv_update_cell(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_update_cell failed: {:?}", res.err());

    let val = mcp_filesystem::actions::csv::csv_read(Some(&json!({"path": &path})), &config).await.unwrap();
    assert_eq!(val["rows"][0][0], "new");
}

#[tokio::test]
async fn test_csv_remove_row() {
    let config = test_config();
    let path = t("rmrow.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["X"],
        "rows": [["a"], ["b"], ["c"]],
    })), &config).await.unwrap();

    let args = json!({ "path": &path, "row": 1 });
    let res = mcp_filesystem::actions::csv::csv_remove_row(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_remove_row failed: {:?}", res.err());

    let val = mcp_filesystem::actions::csv::csv_read(Some(&json!({"path": &path})), &config).await.unwrap();
    assert_eq!(val["totalRows"], 2);
    assert_eq!(val["rows"][0][0], "a");
    assert_eq!(val["rows"][1][0], "c");
}

#[tokio::test]
async fn test_csv_add_column() {
    let config = test_config();
    let path = t("addcol.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["Name"],
        "rows": [["Alice"], ["Bob"]],
    })), &config).await.unwrap();

    let args = json!({
        "path": &path, "column": "Age", "defaultValue": "0",
    });
    let res = mcp_filesystem::actions::csv::csv_add_column(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_add_column failed: {:?}", res.err());

    let val = mcp_filesystem::actions::csv::csv_read(Some(&json!({"path": &path})), &config).await.unwrap();
    assert_eq!(val["headers"].as_array().unwrap().len(), 2);
    assert_eq!(val["rows"][0][1], "0");
}

#[tokio::test]
async fn test_csv_remove_column() {
    let config = test_config();
    let path = t("rmcol.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["A", "B", "C"],
        "rows": [["1", "2", "3"]],
    })), &config).await.unwrap();

    let args = json!({ "path": &path, "column": "B" });
    let res = mcp_filesystem::actions::csv::csv_remove_column(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_remove_column failed: {:?}", res.err());

    let val = mcp_filesystem::actions::csv::csv_read(Some(&json!({"path": &path})), &config).await.unwrap();
    assert_eq!(val["headers"].as_array().unwrap().len(), 2);
    assert_eq!(val["rows"][0].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_csv_rename_column() {
    let config = test_config();
    let path = t("renamecol.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["Old"],
        "rows": [["data"]],
    })), &config).await.unwrap();

    let args = json!({ "path": &path, "oldName": "Old", "newName": "New" });
    let res = mcp_filesystem::actions::csv::csv_rename_column(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_rename_column failed: {:?}", res.err());

    let val = mcp_filesystem::actions::csv::csv_read(Some(&json!({"path": &path})), &config).await.unwrap();
    assert_eq!(val["headers"][0], "New");
}

#[tokio::test]
async fn test_csv_read_column_values_range_basic() {
    let config = test_config();
    let path = t("colvals.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["Name", "Age"],
        "rows": [["Alice", "30"], ["Bob", "25"], ["Carol", "35"]],
    })), &config).await.unwrap();

    let args = json!({ "path": &path, "column": "Name" });
    let res = mcp_filesystem::actions::csv::csv_read_column_values_range(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_read_column_values failed: {:?}", res.err());
    let val = res.unwrap();
    assert_eq!(val["column"], "Name");
    assert_eq!(val["values"].as_array().unwrap().len(), 3);
    assert_eq!(val["values"][0], "Alice");
    assert_eq!(val["values"][2], "Carol");
    assert_eq!(val["totalRows"], 3);
}

#[tokio::test]
async fn test_csv_read_column_values_range_with_bounds() {
    let config = test_config();
    let path = t("colvals_range.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["Name", "Age"],
        "rows": [["Alice", "30"], ["Bob", "25"], ["Carol", "35"], ["Dave", "40"]],
    })), &config).await.unwrap();

    let args = json!({ "path": &path, "column": "Age", "start": 1, "end": 3 });
    let res = mcp_filesystem::actions::csv::csv_read_column_values_range(Some(&args), &config).await.unwrap();
    assert_eq!(res["values"].as_array().unwrap().len(), 2);
    assert_eq!(res["values"][0], "25");
    assert_eq!(res["values"][1], "35");
    assert_eq!(res["start"], 1);
    assert_eq!(res["end"], 3);
}

#[tokio::test]
async fn test_csv_read_row_range_default() {
    let config = test_config();
    let path = t("readrow.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["X", "Y"],
        "rows": [["a", "1"], ["b", "2"], ["c", "3"]],
    })), &config).await.unwrap();

    let args = json!({ "path": &path });
    let res = mcp_filesystem::actions::csv::csv_read_row_range(Some(&args), &config).await;
    assert!(res.is_ok(), "csv_read_row failed: {:?}", res.err());
    let val = res.unwrap();
    assert_eq!(val["headers"].as_array().unwrap().len(), 2);
    assert_eq!(val["rows"].as_array().unwrap().len(), 1);
    assert_eq!(val["rows"][0][0], "a");
    assert_eq!(val["start"], 0);
    assert_eq!(val["end"], 1);
}

#[tokio::test]
async fn test_csv_read_row_range_with_bounds() {
    let config = test_config();
    let path = t("readrow_range.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["X"],
        "rows": [["r0"], ["r1"], ["r2"], ["r3"], ["r4"]],
    })), &config).await.unwrap();

    let args = json!({ "path": &path, "start": 2, "end": 4 });
    let res = mcp_filesystem::actions::csv::csv_read_row_range(Some(&args), &config).await.unwrap();
    assert_eq!(res["rows"].as_array().unwrap().len(), 2);
    assert_eq!(res["rows"][0][0], "r2");
    assert_eq!(res["rows"][1][0], "r3");
    assert_eq!(res["start"], 2);
    assert_eq!(res["end"], 4);
}

#[tokio::test]
async fn test_csv_read_column_values_range_exceeds_limit() {
    let config = test_config();
    let path = t("colvals_too_big.csv");
    let rows: Vec<Vec<String>> = (0..1001).map(|i| vec![i.to_string()]).collect();
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["N"], "rows": rows,
    })), &config).await.unwrap();

    let args = json!({ "path": &path, "column": "N", "start": 0, "end": 1001 });
    let res = mcp_filesystem::actions::csv::csv_read_column_values_range(Some(&args), &config).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Range too large"));
}

#[tokio::test]
async fn test_csv_read_row_range_exceeds_limit() {
    let config = test_config();
    let path = t("row_too_big.csv");
    let rows: Vec<Vec<String>> = (0..101).map(|i| vec![i.to_string()]).collect();
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["N"], "rows": rows,
    })), &config).await.unwrap();

    let args = json!({ "path": &path, "start": 0, "end": 101 });
    let res = mcp_filesystem::actions::csv::csv_read_row_range(Some(&args), &config).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Range too large"));
}

#[tokio::test]
async fn test_csv_read_range_invalid_order() {
    let config = test_config();
    let path = t("range_invalid.csv");
    mcp_filesystem::actions::csv::csv_create(Some(&json!({
        "path": &path, "headers": ["X"],
        "rows": [["a"], ["b"]],
    })), &config).await.unwrap();

    let args = json!({ "path": &path, "column": "X", "start": 3, "end": 1 });
    let res = mcp_filesystem::actions::csv::csv_read_column_values_range(Some(&args), &config).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("must be >= start"));
}

// ────────────────────────────
//  Hash
// ────────────────────────────

#[tokio::test]
async fn test_hash_file_sha256() {
    let config = test_config();
    fs::write(t("hash_me.txt"), "hello").unwrap();

    let args = json!({ "path": t("hash_me.txt"), "algorithm": "sha256" });
    let res = mcp_filesystem::actions::files::hash_file(Some(&args), &config).await;
    assert!(res.is_ok(), "hash_file failed: {:?}", res.err());
    let val = res.unwrap();
    // sha256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
    assert_eq!(val["hash"], "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
}

#[tokio::test]
async fn test_hash_file_sha512() {
    let config = test_config();
    fs::write(t("hash_512.txt"), "hello").unwrap();
    let args = json!({ "path": t("hash_512.txt"), "algorithm": "sha512" });
    let res = mcp_filesystem::actions::files::hash_file(Some(&args), &config).await.unwrap();
    assert_eq!(res["hash"].as_str().unwrap().len(), 128);
}

#[tokio::test]
async fn test_hash_file_blake3() {
    let config = test_config();
    fs::write(t("hash_b3.txt"), "hello").unwrap();
    let args = json!({ "path": t("hash_b3.txt"), "algorithm": "blake3" });
    let res = mcp_filesystem::actions::files::hash_file(Some(&args), &config).await.unwrap();
    assert_eq!(res["hash"].as_str().unwrap().len(), 64);
}

// ────────────────────────────
//  Edge Cases
// ────────────────────────────

#[tokio::test]
async fn test_empty_file() {
    let config = test_config();
    fs::write(t("empty.txt"), "").unwrap();

    let args = json!({ "path": t("empty.txt") });
    let res = mcp_filesystem::actions::files::read_text_file(Some(&args), &config).await;
    assert!(res.is_ok(), "empty file read failed: {:?}", res.err());
    let val = res.unwrap();
    assert_eq!(val["content"], "");
    assert_eq!(val["size"], 0);
    assert_eq!(val["totalLines"], 0);
}

#[tokio::test]
async fn test_read_nonexistent_file_fails() {
    let config = test_config();
    let args = json!({ "path": t("nope.txt") });
    let res = mcp_filesystem::actions::files::read_text_file(Some(&args), &config).await;
    assert!(res.is_err());
    let err = format!("{}", res.unwrap_err());
    assert!(err.contains("does not exist"));
}

#[tokio::test]
async fn test_create_existing_file_without_overwrite_fails() {
    let config = test_config();
    let path = t("existing.csv");
    fs::write(&path, "original").unwrap();

    let args = json!({ "path": &path, "headers": ["a"], "overwrite": false });
    let res = mcp_filesystem::actions::csv::csv_create(Some(&args), &config).await;
    assert!(res.is_err(), "Creating existing file without overwrite should fail");
}

#[tokio::test]
async fn test_generate_key_aes() {
    let config = test_config();
    let args = json!({ "algorithm": "aes-256" });
    let res = mcp_filesystem::actions::crypto::generate_key(Some(&args), &config).await;
    assert!(res.is_ok(), "generate_key AES failed: {:?}", res.err());
    let val = res.unwrap();
    assert!(val["key"].as_str().unwrap().len() == 64); // 32 bytes as hex
}

#[tokio::test]
async fn test_generate_key_rsa() {
    let config = test_config();
    let args = json!({ "algorithm": "rsa-2048" });
    let res = mcp_filesystem::actions::crypto::generate_key(Some(&args), &config).await;
    assert!(res.is_ok(), "generate_key RSA failed: {:?}", res.err());
    let val = res.unwrap();
    assert!(val["publicKey"].as_str().unwrap().contains("BEGIN"));
    assert!(val["privateKey"].as_str().unwrap().contains("BEGIN"));
}

// ────────────────────────────
//  Disk Usage / Directory Tree
// ────────────────────────────

#[tokio::test]
async fn test_disk_usage() {
    let config = test_config();
    fs::write(t("du_file.txt"), "some data").unwrap();
    let args = json!({ "path": t("") });
    let res = mcp_filesystem::actions::files::get_disk_usage(Some(&args), &config).await;
    assert!(res.is_ok(), "get_disk_usage failed: {:?}", res.err());
    let val = res.unwrap();
    assert!(val["totalSize"].as_u64().unwrap() > 0);
    assert!(val["fileCount"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_directory_tree() {
    let config = test_config();
    fs::create_dir_all(t("tree/sub")).unwrap();
    fs::write(t("tree/a.txt"), "a").unwrap();
    let args = json!({ "path": t("tree") });
    let res = mcp_filesystem::actions::files::directory_tree(Some(&args), &config).await;
    assert!(res.is_ok(), "directory_tree failed: {:?}", res.err());
}

// ────────────────────────────
//  Read File Range
// ────────────────────────────

#[tokio::test]
async fn test_read_file_range() {
    let config = test_config();
    fs::write(t("range.txt"), "0123456789").unwrap();

    let args = json!({ "path": t("range.txt"), "offset": 3, "length": 4 });
    let res = mcp_filesystem::actions::files::read_file_range(Some(&args), &config).await;
    assert!(res.is_ok(), "read_file_range failed: {:?}", res.err());
    let val = res.unwrap();
    assert_eq!(val["content"], "3456");
    assert_eq!(val["length"], 4);
}

// ────────────────────────────
//  Read-Only Mode
// ────────────────────────────

#[tokio::test]
async fn test_readonly_mode_blocks_write() {
    let mut config = test_config();
    config.server.access_mode = mcp_filesystem::config::AccessMode::ReadOnly;

    let req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        method: "tools/call".into(),
        params: Some(json!({
            "name": "write_file",
            "arguments": { "path": t("readonly_test.txt"), "content": "should not write" }
        })),
        id: Some(json!(1)),
    };

    let res = process_request(&req, &config).await;
    assert!(res.is_err(), "Write should fail in read-only mode");
    match res {
        Err(e) => assert!(
            e.to_string().contains("not allowed in read-only mode"),
            "Unexpected error: {e}"
        ),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn test_readonly_mode_allows_read() {
    let mut config = test_config();
    config.server.access_mode = mcp_filesystem::config::AccessMode::ReadOnly;
    fs::write(t("readonly_read_test.txt"), "readable content").unwrap();

    let req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        method: "tools/call".into(),
        params: Some(json!({
            "name": "read_text_file",
            "arguments": { "path": t("readonly_read_test.txt") }
        })),
        id: Some(json!(2)),
    };

    let res = process_request(&req, &config).await;
    assert!(res.is_ok(), "Read should succeed in read-only mode: {:?}", res.err());
}

#[tokio::test]
async fn test_readonly_mode_allows_tools_list() {
    let config = test_config();
    let req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        method: "tools/list".into(),
        params: None,
        id: Some(json!(3)),
    };

    let res = process_request(&req, &config).await;
    assert!(res.is_ok(), "tools/list should always succeed");
}
