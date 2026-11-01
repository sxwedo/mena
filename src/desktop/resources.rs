//! Desktop projections of catalog-owned resources. Reads and probes stay in
//! the catalogs; this module only assigns stable identities and display fields.

use std::path::Path;

use anyhow::{Context, Result};

use super::{DesktopField, DesktopResource, DesktopResourceDetail, DesktopResourceKind};
use crate::mcp::{McpCatalog, McpDetail, McpRegistration};
use crate::memory::{MemoryCatalog, MemoryFile};
use crate::skill::{AgentSkill, SkillCatalog};

pub(super) fn list(
    home: &Path,
    workspace: Option<&Path>,
    kind: DesktopResourceKind,
) -> Result<Vec<DesktopResource>> {
    Ok(match kind {
        DesktopResourceKind::Skills => SkillCatalog::scan(Some(home), workspace)?
            .skills()
            .iter()
            .map(skill)
            .collect(),
        DesktopResourceKind::Mcp => McpCatalog::scan(Some(home), workspace)?
            .registrations()
            .iter()
            .map(mcp)
            .collect(),
        DesktopResourceKind::Memories => MemoryCatalog::scan(Some(home), workspace)?
            .files()
            .iter()
            .map(memory)
            .collect(),
    })
}

pub(super) fn detail(
    home: &Path,
    workspace: Option<&Path>,
    kind: DesktopResourceKind,
    id: &str,
) -> Result<DesktopResourceDetail> {
    match kind {
        DesktopResourceKind::Skills => {
            let catalog = SkillCatalog::scan(Some(home), workspace)?;
            let selected = catalog
                .skills()
                .iter()
                .find(|entry| skill(entry).id == id)
                .context("Skill 已不存在，请刷新列表。")?;
            let loaded = catalog.entry(selected, &selected.path)?;
            Ok(DesktopResourceDetail {
                resource: skill(selected),
                fields: vec![
                    DesktopField::new("Provider", &selected.provider),
                    DesktopField::new("作用范围", &selected.scope),
                    DesktopField::new("入口", selected.path.display().to_string()),
                    DesktopField::new("触发条件", selected.triggers.join(", ")),
                ],
                content: loaded.detail.content,
                probe_status: None,
            })
        }
        DesktopResourceKind::Memories => {
            let catalog = MemoryCatalog::scan(Some(home), workspace)?;
            let selected = catalog
                .files()
                .iter()
                .find(|entry| memory(entry).id == id)
                .context("记忆文件已不存在，请刷新列表。")?;
            Ok(DesktopResourceDetail {
                resource: memory(selected),
                fields: vec![
                    DesktopField::new("Provider", &selected.provider),
                    DesktopField::new("作用范围", &selected.scope),
                    DesktopField::new("位置", selected.path.display().to_string()),
                    DesktopField::new("大小", format!("{} bytes", selected.size_bytes)),
                ],
                content: catalog.read(&selected.path)?,
                probe_status: None,
            })
        }
        DesktopResourceKind::Mcp => {
            let catalog = McpCatalog::scan(Some(home), workspace)?;
            let selected = catalog
                .registrations()
                .iter()
                .find(|entry| mcp(entry).id == id)
                .context("MCP 注册已不存在，请刷新列表。")?;
            mcp_detail(&McpDetail {
                registration: selected.clone(),
                probe: None,
            })
        }
    }
}

pub(super) fn probe(
    home: &Path,
    workspace: Option<&Path>,
    id: &str,
) -> Result<DesktopResourceDetail> {
    let catalog = McpCatalog::scan(Some(home), workspace)?;
    let selected = catalog
        .registrations()
        .iter()
        .find(|entry| mcp(entry).id == id)
        .context("MCP 注册已不存在，请刷新列表。")?;
    // Revalidates the full registration identity and its native connection.
    mcp_detail(&catalog.inspect_current_registration_with_probe(selected, 8)?)
}

fn identity(provider: &str, scope: &str, name: &str, path: &Path) -> String {
    // The name's byte length disambiguates delimiters in names and paths.
    format!("{provider}:{scope}:{}:{name}{}", name.len(), path.display())
}

fn skill(entry: &AgentSkill) -> DesktopResource {
    DesktopResource {
        id: identity(&entry.provider, &entry.scope, &entry.name, &entry.path),
        name: entry.name.clone(),
        provider: entry.provider.clone(),
        scope: entry.scope.clone(),
        description: entry.description.clone().unwrap_or_default(),
        path: entry.path.display().to_string(),
        status: if entry.valid {
            "可用"
        } else {
            "无法预览"
        }
        .to_owned(),
    }
}

fn memory(entry: &MemoryFile) -> DesktopResource {
    DesktopResource {
        id: identity(&entry.provider, &entry.scope, &entry.name, &entry.path),
        name: entry.name.clone(),
        provider: entry.provider.clone(),
        scope: entry.scope.clone(),
        description: entry.location.clone(),
        path: entry.path.display().to_string(),
        status: if entry.scope == "user" {
            "用户记忆"
        } else {
            "项目记忆"
        }
        .to_owned(),
    }
}

fn mcp(entry: &McpRegistration) -> DesktopResource {
    DesktopResource {
        id: identity(&entry.provider, &entry.scope, &entry.name, &entry.source),
        name: entry
            .display_name
            .clone()
            .unwrap_or_else(|| entry.name.clone()),
        provider: entry.provider.clone(),
        scope: entry.scope.clone(),
        description: entry
            .description
            .clone()
            .unwrap_or_else(|| entry.transport.as_str().to_owned()),
        path: entry.source.display().to_string(),
        status: if !entry.valid {
            "配置无效"
        } else if entry.enabled {
            "未探测"
        } else {
            "已禁用"
        }
        .to_owned(),
    }
}

fn mcp_detail(detail: &McpDetail) -> Result<DesktopResourceDetail> {
    let entry = &detail.registration;
    let mut fields = vec![
        DesktopField::new("客户端", &entry.provider),
        DesktopField::new("作用范围", &entry.scope),
        DesktopField::new("传输方式", entry.transport.as_str()),
        DesktopField::new("配置来源", entry.source.display().to_string()),
    ];
    if let Some(probe) = &detail.probe {
        fields.push(DesktopField::new("探测结果", probe.status.as_str()));
        fields.push(DesktopField::new("工具数量", probe.tools.len().to_string()));
        fields.push(DesktopField::new(
            "耗时",
            format!("{} ms", probe.duration_ms),
        ));
    }
    Ok(DesktopResourceDetail {
        resource: mcp(entry),
        fields,
        // Only the catalog's redacted public metadata is serializable here.
        content: serde_json::to_string_pretty(detail)?,
        probe_status: detail
            .probe
            .as_ref()
            .map(|probe| probe.status.as_str().to_owned()),
    })
}
