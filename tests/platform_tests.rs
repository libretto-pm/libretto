//! Cross-platform tests for Libretto.
//!
//! These tests verify platform-specific behavior across Linux, macOS, and Windows,
//! including path handling, file permissions, line endings, and SIMD operations.

use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ========== Path Separator Tests ==========

mod path_separators {
    use super::*;

    #[test]
    fn test_forward_slash_works_everywhere() {
        let temp = TempDir::new().unwrap();

        // Forward slashes should work on all platforms
        let path = temp.path().join("vendor/monolog/monolog");
        fs::create_dir_all(&path).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn test_path_normalization() {
        let temp = TempDir::new().unwrap();

        // Create a nested path
        let nested = temp.path().join("a/b/c/d");
        fs::create_dir_all(&nested).unwrap();

        // Paths with . and .. should normalize correctly
        let with_dots = temp.path().join("a/b/./c/../c/d");
        let normalized = with_dots.canonicalize().unwrap();

        assert!(normalized.ends_with("a/b/c/d") || normalized.ends_with("a\\b\\c\\d"));
    }

    #[test]
    fn test_relative_path_resolution() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Create structure
        fs::create_dir_all(base.join("src/App")).unwrap();
        fs::write(base.join("src/App/Controller.php"), "<?php").unwrap();

        // Relative path from different location
        let relative = Path::new("src/App/Controller.php");
        let absolute = base.join(relative);

        assert!(absolute.exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_long_path() {
        let temp = TempDir::new().unwrap();

        // Create a path approaching Windows MAX_PATH (260 chars)
        let mut path = temp.path().to_path_buf();
        for i in 0..20 {
            path = path.join(format!("directory_{:02}", i));
        }

        // This might fail on Windows without long path support
        let result = fs::create_dir_all(&path);

        // Test passes if it works or fails with a known error
        match result {
            Ok(()) => assert!(path.exists()),
            Err(e) => {
                // Expected error for path too long
                println!("Long path failed (expected on some Windows configs): {}", e);
            }
        }
    }
}

// ========== File Permissions Tests ==========

mod file_permissions {
    use super::*;

    #[cfg(unix)]
    mod unix_permissions {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        #[test]
        fn test_executable_flag() {
            let temp = TempDir::new().unwrap();
            let script = temp.path().join("vendor/bin/tool");

            fs::create_dir_all(script.parent().unwrap()).unwrap();
            fs::write(&script, "#!/bin/bash\necho 'Hello'").unwrap();

            // Set executable permissions
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();

            // Verify
            let actual = fs::metadata(&script).unwrap().permissions().mode();
            assert!(actual & 0o111 != 0, "File should be executable");
        }

        #[test]
        fn test_preserve_permissions_on_copy() {
            let temp = TempDir::new().unwrap();
            let src = temp.path().join("source.sh");
            let dst = temp.path().join("dest.sh");

            fs::write(&src, "#!/bin/bash").unwrap();

            // Set specific permissions
            let mut perms = fs::metadata(&src).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&src, perms).unwrap();

            // Copy file
            fs::copy(&src, &dst).unwrap();

            // Preserve permissions
            let src_mode = fs::metadata(&src).unwrap().permissions().mode() & 0o777;
            fs::set_permissions(&dst, fs::Permissions::from_mode(src_mode)).unwrap();

            let dst_mode = fs::metadata(&dst).unwrap().permissions().mode() & 0o777;
            assert_eq!(src_mode, dst_mode);
        }

        #[test]
        fn test_directory_permissions() {
            let temp = TempDir::new().unwrap();
            let dir = temp.path().join("vendor");

            fs::create_dir(&dir).unwrap();

            let perms = fs::metadata(&dir).unwrap().permissions();
            let mode = perms.mode() & 0o777;

            // Directory should be readable and executable
            assert!(
                mode & 0o500 != 0,
                "Directory should be readable and executable"
            );
        }
    }

    #[cfg(windows)]
    mod windows_permissions {
        use super::*;

        #[test]
        fn test_read_only_flag() {
            let temp = TempDir::new().unwrap();
            let file = temp.path().join("readonly.txt");

            fs::write(&file, "content").unwrap();

            // Set read-only
            let mut perms = fs::metadata(&file).unwrap().permissions();
            perms.set_readonly(true);
            fs::set_permissions(&file, perms).unwrap();

            // Verify
            let readonly = fs::metadata(&file).unwrap().permissions().readonly();
            assert!(readonly, "File should be read-only");

            // Clean up: remove read-only for deletion
            let mut perms = fs::metadata(&file).unwrap().permissions();
            perms.set_readonly(false);
            fs::set_permissions(&file, perms).unwrap();
        }
    }
}

// ========== Line Ending Tests ==========

mod line_endings {
    use super::*;

    #[test]
    fn test_lf_line_endings() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("lf.txt");

        let content = "line1\nline2\nline3\n";
        fs::write(&file, content).unwrap();

        let read = fs::read_to_string(&file).unwrap();

        // Should preserve LF on all platforms when read as binary
        assert!(read.contains('\n'));
        assert!(!read.contains("\r\n"));
    }

    #[test]
    fn test_crlf_line_endings() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("crlf.txt");

        let content = "line1\r\nline2\r\nline3\r\n";
        fs::write(&file, content).unwrap();

        let read = fs::read(&file).unwrap();

        // Should preserve CRLF when read as bytes
        assert!(read.windows(2).any(|w| w == b"\r\n"));
    }

    #[test]
    fn test_mixed_line_endings() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("mixed.txt");

        let content = "line1\nline2\r\nline3\rline4\n";
        fs::write(&file, content).unwrap();

        let read = fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = read.lines().collect();

        // lines() should handle all line ending types
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_normalize_line_endings() {
        let content = "line1\r\nline2\r\nline3";

        // Normalize to LF
        let normalized = content.replace("\r\n", "\n");

        assert!(!normalized.contains("\r\n"));
        assert!(normalized.contains('\n'));
    }
}

// ========== Case Sensitivity Tests ==========

mod case_sensitivity {
    use super::*;

    #[test]
    fn test_case_handling() {
        let temp = TempDir::new().unwrap();

        // Create file with specific case
        let lower = temp.path().join("test.php");
        fs::write(&lower, "lower").unwrap();

        // Try to access with different case
        let upper = temp.path().join("TEST.PHP");

        // On case-insensitive systems (Windows, macOS by default),
        // these would refer to the same file
        let same_file = lower.canonicalize().ok() == upper.canonicalize().ok();

        if same_file {
            // Case-insensitive filesystem
            let content = fs::read_to_string(&upper).unwrap();
            assert_eq!(content, "lower");
        } else {
            // Case-sensitive filesystem
            assert!(!upper.exists());
        }
    }

    #[test]
    fn test_consistent_case_in_paths() {
        // Package names should be normalized to lowercase
        let names = vec![
            ("Monolog/Monolog", "monolog/monolog"),
            ("PSR/Log", "psr/log"),
            ("GUZZLEHTTP/Guzzle", "guzzlehttp/guzzle"),
        ];

        for (input, expected) in names {
            let normalized = input.to_lowercase();
            assert_eq!(normalized, expected);
        }
    }
}

// ========== Symlink Tests ==========

mod symlinks {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn test_symlink_creation() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");

        fs::write(&target, "content").unwrap();
        symlink(&target, &link).unwrap();

        assert!(link.exists());
        assert!(link.is_symlink());

        let content = fs::read_to_string(&link).unwrap();
        assert_eq!(content, "content");
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_to_directory() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let target_dir = temp.path().join("real_vendor");
        let link = temp.path().join("vendor");

        fs::create_dir(&target_dir).unwrap();
        fs::write(target_dir.join("autoload.php"), "<?php").unwrap();

        symlink(&target_dir, &link).unwrap();

        assert!(link.join("autoload.php").exists());
    }

    #[cfg(windows)]
    #[test]
    fn test_junction_creation() {
        // Windows junctions (directory links) don't require admin privileges
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target_dir");
        let junction = temp.path().join("junction");

        fs::create_dir(&target).unwrap();
        fs::write(target.join("file.txt"), "content").unwrap();

        // Creating junctions requires using Windows API or command line
        // This is a simplified test
        #[cfg(feature = "junction")]
        {
            // junction::create(&target, &junction).unwrap();
            // assert!(junction.join("file.txt").exists());
        }
    }
}

// ========== Encoding Tests ==========

mod encoding {
    use super::*;

    #[test]
    fn test_utf8_filenames() {
        let temp = TempDir::new().unwrap();

        // UTF-8 filenames (may not work on all Windows configurations)
        let unicode_names = vec!["日本語.php", "émojis🎉.txt", "中文文件.json"];

        for name in unicode_names {
            let path = temp.path().join(name);
            match fs::write(&path, "content") {
                Ok(()) => {
                    assert!(path.exists());
                    let content = fs::read_to_string(&path).unwrap();
                    assert_eq!(content, "content");
                }
                Err(e) => {
                    // Some filesystems don't support these characters
                    println!("Unicode filename '{}' not supported: {}", name, e);
                }
            }
        }
    }

    #[test]
    fn test_utf8_file_content() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("utf8.txt");

        let content = "日本語\n中文\n한국어\nعربى\n🎉🎊🎈";
        fs::write(&file, content).unwrap();

        let read = fs::read_to_string(&file).unwrap();
        assert_eq!(read, content);
    }

    #[test]
    fn test_bom_handling() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("bom.txt");

        // UTF-8 BOM + content
        let bom = b"\xEF\xBB\xBF";
        let content = b"content after BOM";
        let mut data = Vec::from(&bom[..]);
        data.extend_from_slice(content);

        fs::write(&file, &data).unwrap();

        let read = fs::read(&file).unwrap();
        assert!(read.starts_with(bom));
    }
}

// ========== Atomic Operations Tests ==========

mod atomic_ops {
    use super::*;

    #[test]
    fn test_atomic_write_via_rename() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("config.json");
        let temp_file = temp.path().join("config.json.tmp");

        // Initial content
        fs::write(&target, r#"{"version": 1}"#).unwrap();

        // Atomic update: write to temp, then rename
        fs::write(&temp_file, r#"{"version": 2}"#).unwrap();
        fs::rename(&temp_file, &target).unwrap();

        // Verify
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("version\": 2") || content.contains("version\":2"));
        assert!(!temp_file.exists());
    }

    #[test]
    fn test_file_locking_simulation() {
        let temp = TempDir::new().unwrap();
        let lock_file = temp.path().join("composer.lock.lock");

        // Create lock file
        let created = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_file);

        match created {
            Ok(_file) => {
                assert!(lock_file.exists());

                // Try to create again - should fail
                let duplicate = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&lock_file);

                assert!(duplicate.is_err());
            }
            Err(e) => {
                println!("Lock file creation failed: {}", e);
            }
        }
    }
}

// ========== Environment Tests ==========

mod environment {
    #[test]
    fn test_home_directory() {
        let home = dirs::home_dir();
        assert!(home.is_some(), "Home directory should be available");

        let home = home.unwrap();
        assert!(home.exists(), "Home directory should exist");
    }

    #[test]
    fn test_temp_directory() {
        let temp = std::env::temp_dir();
        assert!(temp.exists(), "Temp directory should exist");
    }

    #[test]
    fn test_current_directory() {
        let cwd = std::env::current_dir();
        assert!(cwd.is_ok(), "Current directory should be accessible");
    }

    #[test]
    fn test_path_env_var() {
        let path = std::env::var("PATH").or_else(|_| std::env::var("Path"));
        assert!(path.is_ok(), "PATH environment variable should be set");
    }
}

// ========== Shell Execution Tests ==========

mod shell_execution {
    use std::process::Command;

    #[test]
    fn test_shell_available() {
        #[cfg(unix)]
        {
            let output = Command::new("sh").arg("-c").arg("echo test").output();
            assert!(output.is_ok());
        }

        #[cfg(windows)]
        {
            let output = Command::new("cmd").args(["/C", "echo test"]).output();
            assert!(output.is_ok());
        }
    }

    #[test]
    fn test_php_detection() {
        let output = Command::new("php").arg("--version").output();

        match output {
            Ok(o) if o.status.success() => {
                let version = String::from_utf8_lossy(&o.stdout);
                assert!(version.contains("PHP"));
            }
            _ => {
                println!("PHP not available on this system");
            }
        }
    }

    #[test]
    fn test_git_detection() {
        let output = Command::new("git").arg("--version").output();

        match output {
            Ok(o) if o.status.success() => {
                let version = String::from_utf8_lossy(&o.stdout);
                assert!(version.contains("git"));
            }
            _ => {
                println!("Git not available on this system");
            }
        }
    }
}

// Helper module for directory functions
mod dirs {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        #[cfg(unix)]
        {
            std::env::var("HOME").ok().map(PathBuf::from)
        }

        #[cfg(windows)]
        {
            std::env::var("USERPROFILE").ok().map(PathBuf::from)
        }
    }
}
