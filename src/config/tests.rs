// SPDX-License-Identifier: GPL-3.0-only
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use serial_test::serial;
use tempfile::NamedTempFile;

use crate::config::env::keys;
use crate::config::model::Config;
use crate::config::test_support::{
    clear_kether_env_vars, remove_env_var, set_env_var, with_env_var, with_isolated_config,
    ISOLATED_CONFIG_PATH,
};

#[test]
fn test_default_config() {
    let config = Config::default();
    assert_eq!(config.l4d2_server_dir, PathBuf::from("/home/steam/l4d2"));
    assert_eq!(config.registry_path, PathBuf::from("registry.json"));
    assert_eq!(config.backend_api_url, "http://localhost:3000/api");
    assert_eq!(config.backend_api_key, None);
    assert_eq!(
        config.local_api_bind,
        SocketAddr::from_str("127.0.0.1:8080").unwrap()
    );
    assert_eq!(config.sync_interval_secs, 300);
    assert_eq!(config.log_level, "info");
    assert_eq!(
        config.l4d2center_index_url,
        "https://l4d2center.com/maps/servers/index.json"
    );
}

#[test]
fn test_addons_dir() {
    let config = Config::default();
    let addons_dir = config.addons_dir();
    assert_eq!(
        addons_dir,
        PathBuf::from("/home/steam/l4d2/left4dead2/addons")
    );
}

#[test]
#[serial]
fn test_load_missing_config_file() {
    with_isolated_config(|| {
        let config = Config::load().unwrap();
        assert_eq!(config.l4d2_server_dir, PathBuf::from("/home/steam/l4d2"));
        assert_eq!(config.registry_path, PathBuf::from("registry.json"));
    });
}

#[test]
#[serial]
fn test_load_from_toml() {
    let temp_file = NamedTempFile::new().unwrap();
    let config_content = r#"
l4d2_server_dir = "/custom/server/path"
registry_path = "/custom/registry.json"
backend_api_url = "http://custom-api.example.com"
backend_api_key = "test-key-123"
local_api_bind = "0.0.0.0:9000"
sync_interval_secs = 600
log_level = "debug"
"#;
    fs::write(temp_file.path(), config_content).unwrap();

    let original_config = std::env::var(keys::CONFIG).ok();
    set_env_var(keys::CONFIG, temp_file.path().to_str().unwrap());

    remove_env_var(keys::L4D2_SERVER_DIR);
    remove_env_var(keys::REGISTRY_PATH);
    remove_env_var(keys::BACKEND_API_URL);
    remove_env_var(keys::BACKEND_API_KEY);
    remove_env_var(keys::LOCAL_API_BIND);
    remove_env_var(keys::SYNC_INTERVAL_SECS);
    remove_env_var(keys::LOG_LEVEL);

    let config = Config::load().unwrap();
    assert_eq!(config.l4d2_server_dir, PathBuf::from("/custom/server/path"));
    assert_eq!(config.registry_path, PathBuf::from("/custom/registry.json"));
    assert_eq!(config.backend_api_url, "http://custom-api.example.com");
    assert_eq!(config.backend_api_key, Some("test-key-123".to_string()));
    assert_eq!(
        config.local_api_bind,
        SocketAddr::from_str("0.0.0.0:9000").unwrap()
    );
    assert_eq!(config.sync_interval_secs, 600);
    assert_eq!(config.log_level, "debug");

    if let Some(val) = original_config {
        set_env_var(keys::CONFIG, &val);
    } else {
        remove_env_var(keys::CONFIG);
    }
}

#[test]
#[serial]
fn test_env_var_overrides() {
    struct Case {
        key: &'static str,
        value: &'static str,
        assert: fn(&Config),
    }

    let cases = [
        Case {
            key: keys::L4D2_SERVER_DIR,
            value: "/env/server/path",
            assert: |config| {
                assert_eq!(config.l4d2_server_dir, PathBuf::from("/env/server/path"));
            },
        },
        Case {
            key: keys::REGISTRY_PATH,
            value: "/env/registry.json",
            assert: |config| {
                assert_eq!(config.registry_path, PathBuf::from("/env/registry.json"));
            },
        },
        Case {
            key: keys::BACKEND_API_URL,
            value: "http://env-api.example.com",
            assert: |config| {
                assert_eq!(config.backend_api_url, "http://env-api.example.com");
            },
        },
        Case {
            key: keys::BACKEND_API_KEY,
            value: "env-key-456",
            assert: |config| {
                assert_eq!(config.backend_api_key, Some("env-key-456".to_string()));
            },
        },
        Case {
            key: keys::LOCAL_API_BIND,
            value: "192.168.1.1:9090",
            assert: |config| {
                assert_eq!(
                    config.local_api_bind,
                    SocketAddr::from_str("192.168.1.1:9090").unwrap()
                );
            },
        },
        Case {
            key: keys::SYNC_INTERVAL_SECS,
            value: "120",
            assert: |config| {
                assert_eq!(config.sync_interval_secs, 120);
            },
        },
        Case {
            key: keys::LOG_LEVEL,
            value: "trace",
            assert: |config| {
                assert_eq!(config.log_level, "trace");
            },
        },
    ];

    for case in cases {
        with_isolated_config(|| {
            with_env_var(case.key, case.value, || {
                let config = Config::load().unwrap();
                (case.assert)(&config);
            });
        });
    }
}

#[test]
fn test_validate_rejects_zero_sync_interval() {
    let (mut config, _dir) = crate::test_helpers::create_test_config();
    config.sync_interval_secs = 0;
    assert!(config.validate().is_err());
}

#[test]
fn test_validate_rejects_invalid_log_level() {
    let (mut config, _dir) = crate::test_helpers::create_test_config();
    config.log_level = "verbose".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn test_validate_accepts_valid_config() {
    let (config, _dir) = crate::test_helpers::create_test_config();
    assert!(config.validate().is_ok());
}
