//! 规则模型定义。

use serde::{Deserialize, Serialize};

/// 一条清理规则
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub homepage: Option<String>,
    pub risk: Risk,
    pub disclaimer: String,
    pub detect: Vec<String>,
    #[serde(rename = "match", default)]
    pub matcher: Match,
    #[serde(
        rename(deserialize = "scope", serialize = "scopes"),
        alias = "scopes",
        default
    )]
    pub scopes: Vec<Scope>,
}

/// 匹配辅助
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Match {
    #[serde(default)]
    pub name_contains: Vec<String>,
    #[serde(default)]
    pub must_have_child: Vec<String>,
}

/// 一个清理范围
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Scope {
    pub id: String,
    pub label: String,
    pub glob: String,
    pub mode: Mode,
    #[serde(default)]
    pub prompt: Option<Prompt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default)]
    pub recycle_granularity: RecycleGranularity,
}

/// 执行模式
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Recycle,
    Quarantine,
    Delete,
    SystemCmd,
}

/// 回收粒度
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecycleGranularity {
    #[default]
    File,
    Directory,
}

/// 风险等级
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Low,
    Medium,
    High,
}

/// 交互提示
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Prompt {
    None,
    Days { default: u32, #[serde(default)] label: Option<String> },
    Bytes { default: u64, #[serde(default)] label: Option<String> },
    Choice { default: String, options: Vec<String>, #[serde(default)] label: Option<String> },
    Confirm { #[serde(default)] label: Option<String> },
}
