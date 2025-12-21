use crate::error::{Result, VideoProcessorError};
use crate::types::ProcessingConfig;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Application configuration loaded from TOML file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub processing: ProcessingConfig,
    
    #[serde(default)]
    pub logging: LoggingConfig,
    
    #[serde(default)]
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    
    #[serde(default = "default_log_to_file")]
    pub log_to_file: bool,
    
    #[serde(default)]
    pub log_dir: Option<PathBuf>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            log_to_file: default_log_to_file(),
            log_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    #[serde(default = "default_checkpoint_enabled")]
    pub checkpoint_enabled: bool,
    
    #[serde(default = "default_auto_resume")]
    pub auto_resume: bool,
    
    #[serde(default = "default_cleanup_on_success")]
    pub cleanup_on_success: bool,
    
    #[serde(default = "default_min_disk_space_gb")]
    pub min_disk_space_gb: u64,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            checkpoint_enabled: default_checkpoint_enabled(),
            auto_resume: default_auto_resume(),
            cleanup_on_success: default_cleanup_on_success(),
            min_disk_space_gb: default_min_disk_space_gb(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_to_file() -> bool {
    true
}

fn default_checkpoint_enabled() -> bool {
    true
}

fn default_auto_resume() -> bool {
    false
}

fn default_cleanup_on_success() -> bool {
    true
}

fn default_min_disk_space_gb() -> u64 {
    10
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            processing: ProcessingConfig::default(),
            logging: LoggingConfig::default(),
            behavior: BehaviorConfig::default(),
        }
    }
}

impl AppConfig {
    /// Load configuration from TOML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let contents = fs::read_to_string(path.as_ref()).map_err(|e| {
            VideoProcessorError::ConfigError(format!("Failed to read config file: {}", e))
        })?;

        let config: AppConfig = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Save configuration to TOML file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let contents = toml::to_string_pretty(self).map_err(|e| {
            VideoProcessorError::ConfigError(format!("Failed to serialize config: {}", e))
        })?;

        fs::write(path.as_ref(), contents).map_err(|e| {
            VideoProcessorError::ConfigError(format!("Failed to write config file: {}", e))
        })?;

        Ok(())
    }

    /// Get default config file path (in user's config directory)
    pub fn default_config_path() -> PathBuf {
        if let Some(config_dir) = dirs::config_dir() {
            config_dir.join("ffmpeg-video-processor").join("config.toml")
        } else {
            PathBuf::from("config.toml")
        }
    }

    /// Create default configuration file if it doesn't exist
    #[allow(dead_code)]
    pub fn ensure_default_config() -> Result<PathBuf> {
        let config_path = Self::default_config_path();

        if !config_path.exists() {
            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    VideoProcessorError::ConfigError(format!("Failed to create config directory: {}", e))
                })?;
            }

            let default_config = Self::default();
            default_config.save_to_file(&config_path)?;
        }

        Ok(config_path)
    }

    /// Load configuration with fallback to default
    pub fn load_or_default() -> Self {
        let config_path = Self::default_config_path();
        
        if config_path.exists() {
            Self::load_from_file(&config_path).unwrap_or_else(|e| {
                eprintln!("Warning: Failed to load config from {:?}: {}", config_path, e);
                eprintln!("Using default configuration");
                Self::default()
            })
        } else {
            Self::default()
        }
    }

    /// Create fast profile configuration
    pub fn with_fast_profile() -> Self {
        Self {
            processing: ProcessingConfig::fast(),
            ..Self::default()
        }
    }

    /// Create balanced profile configuration
    pub fn with_balanced_profile() -> Self {
        Self {
            processing: ProcessingConfig::balanced(),
            ..Self::default()
        }
    }

    /// Create quality profile configuration
    pub fn with_quality_profile() -> Self {
        Self {
            processing: ProcessingConfig::quality(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.processing.target_resolution, (3840, 2160));  // 4K
        assert_eq!(config.processing.target_fps, 25);
        assert_eq!(config.logging.level, "info");
        assert!(config.behavior.checkpoint_enabled);
    }

    #[test]
    fn test_save_and_load_config() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let config = AppConfig::default();
        config.save_to_file(path).unwrap();

        let loaded = AppConfig::load_from_file(path).unwrap();
        assert_eq!(loaded.processing.target_fps, config.processing.target_fps);
        assert_eq!(loaded.processing.video_crf, config.processing.video_crf);
    }

    #[test]
    fn test_profile_configurations() {
        let fast = AppConfig::with_fast_profile();
        assert_eq!(fast.processing.video_preset, "faster");
        assert_eq!(fast.processing.video_crf, 24);

        let quality = AppConfig::with_quality_profile();
        assert_eq!(quality.processing.video_preset, "slow");
        assert_eq!(quality.processing.video_crf, 18);
    }
}
