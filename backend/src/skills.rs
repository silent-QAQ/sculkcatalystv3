use crate::{AppState, SkillInfo};

pub(crate) const MINECRAFT_PLUGIN_SKILL_ID: &str = "develop-minecraft-server-plugin";
pub(crate) const MINECRAFT_PLUGIN_SKILL_VERSION: &str = "bundled-2026.07";
pub(crate) const MINECRAFT_SERVER_SKILL_ID: &str = "minecraft-server-operations";
pub(crate) const MINECRAFT_SERVER_SKILL_VERSION: &str = "bundled-2026.08";

const SKILL_MD: &str = include_str!("../resources/skills/develop-minecraft-server-plugin/SKILL.md");
const REFERENCES_INDEX: &str =
    include_str!("../resources/skills/develop-minecraft-server-plugin/references/index.md");
const REFERENCES_WORKFLOW: &str = include_str!(
    "../resources/skills/develop-minecraft-server-plugin/references/workflow-and-planning.md"
);
const REFERENCES_TESTING: &str = include_str!(
    "../resources/skills/develop-minecraft-server-plugin/references/testing-and-delivery.md"
);
const REFERENCES_PLATFORM: &str = include_str!(
    "../resources/skills/develop-minecraft-server-plugin/references/platform-and-versions.md"
);
const REFERENCES_DEPENDENCIES: &str = include_str!(
    "../resources/skills/develop-minecraft-server-plugin/references/dependency-selection.md"
);
const REFERENCES_API: &str = include_str!(
    "../resources/skills/develop-minecraft-server-plugin/references/plugin-api-handbook.md"
);
const REFERENCES_GUI: &str = include_str!(
    "../resources/skills/develop-minecraft-server-plugin/references/gui-and-configuration.md"
);
const REFERENCES_GUI_EDITOR: &str =
    include_str!("../resources/skills/develop-minecraft-server-plugin/references/gui-editor.md");
const REFERENCES_DIALOG_EDITOR: &str =
    include_str!("../resources/skills/develop-minecraft-server-plugin/references/dialog-editor.md");
const REFERENCES_LOCAL_API: &str = include_str!(
    "../resources/skills/develop-minecraft-server-plugin/references/local-api-cache.md"
);
const SERVER_SKILL_MD: &str =
    include_str!("../resources/skills/minecraft-server-operations/SKILL.md");
const SERVER_REFERENCES_INDEX: &str =
    include_str!("../resources/skills/minecraft-server-operations/references/index.md");
const SERVER_REFERENCES_FOUNDATIONS: &str =
    include_str!("../resources/skills/minecraft-server-operations/references/foundations.md");
const SERVER_REFERENCES_PLUGINS: &str = include_str!(
    "../resources/skills/minecraft-server-operations/references/plugins-and-content.md"
);
const SERVER_REFERENCES_WORLDS: &str =
    include_str!("../resources/skills/minecraft-server-operations/references/worlds-and-modes.md");
const SERVER_REFERENCES_PERFORMANCE: &str =
    include_str!("../resources/skills/minecraft-server-operations/references/performance.md");
const SERVER_REFERENCES_RECOVERY: &str = include_str!(
    "../resources/skills/minecraft-server-operations/references/recovery-and-migration.md"
);
const SERVER_ANALYSIS_SKILL_MD: &str =
    include_str!("../resources/skills/minecraft-server-operations/analysis-SKILL.md");
const SERVER_ANALYSIS_DIAGNOSTICS: &str = include_str!(
    "../resources/skills/minecraft-server-operations/references/analysis/diagnostics.md"
);
const SERVER_ANALYSIS_SAFETY: &str =
    include_str!("../resources/skills/minecraft-server-operations/references/analysis/safety.md");
const SERVER_ANALYSIS_PLUGINS: &str = include_str!(
    "../resources/skills/minecraft-server-operations/references/analysis/plugin-ecosystem.md"
);
const SERVER_ANALYSIS_APPROVAL: &str = include_str!(
    "../resources/skills/minecraft-server-operations/references/analysis/approval-policy.md"
);

pub(crate) fn builtin_skill_info() -> SkillInfo {
    SkillInfo {
        id: MINECRAFT_PLUGIN_SKILL_ID.into(),
        name: "Minecraft 插件开发".into(),
        description: "面向 Paper、Folia、Spigot/Bukkit 的插件规划、开发、测试与交付工作流。".into(),
        source: "builtin".into(),
        enabled: true,
        version: MINECRAFT_PLUGIN_SKILL_VERSION.into(),
    }
}

pub(crate) fn builtin_server_skill_info() -> SkillInfo {
    SkillInfo {
        id: MINECRAFT_SERVER_SKILL_ID.into(),
        name: "Minecraft 服务器运维".into(),
        description: "服务器核心、插件/模组、配置、性能、安全、世界、迁移与恢复决策。".into(),
        source: "builtin".into(),
        enabled: true,
        version: MINECRAFT_SERVER_SKILL_VERSION.into(),
    }
}

/// 给旧 state.json 补上随程序发布的 Skill，同时保留用户已经设置的启停状态。
pub(crate) fn ensure_bundled_skill(skills: &mut Vec<SkillInfo>) -> bool {
    if skills
        .iter()
        .any(|skill| skill.id == MINECRAFT_PLUGIN_SKILL_ID)
    {
        return false;
    }
    skills.push(builtin_skill_info());
    true
}

/// 给旧 state.json 补上服务器运维 Skill，同时保留用户已经设置的启停状态。
pub(crate) fn ensure_bundled_server_skill(skills: &mut Vec<SkillInfo>) -> bool {
    if skills
        .iter()
        .any(|skill| skill.id == MINECRAFT_SERVER_SKILL_ID)
    {
        return false;
    }
    skills.push(builtin_server_skill_info());
    true
}

pub(crate) async fn is_enabled(state: &AppState) -> bool {
    let data = state.inner.read().await;
    data.skills
        .iter()
        .find(|skill| skill.id == MINECRAFT_PLUGIN_SKILL_ID)
        .map(|skill| skill.enabled)
        .unwrap_or(false)
}

pub(crate) async fn server_is_enabled(state: &AppState) -> bool {
    let data = state.inner.read().await;
    data.skills
        .iter()
        .find(|skill| skill.id == MINECRAFT_SERVER_SKILL_ID)
        .map(|skill| skill.enabled)
        .unwrap_or(false)
}

pub(crate) fn is_minecraft_server_request(query: &str) -> bool {
    let lower = query.to_ascii_lowercase();
    if ["browser plugin", "wordpress plugin", "vscode extension"]
        .iter()
        .any(|term| lower.contains(term))
        || query.contains("浏览器插件")
        || query.contains("浏览器扩展")
        || lower.contains("wordpress 插件")
        || lower.contains("chrome 插件")
        || lower.contains("vscode 插件")
    {
        return false;
    }
    [
        "minecraft",
        "paper",
        "purpur",
        "pufferfish",
        "leaves",
        "folia",
        "fabric",
        "forge",
        "neoforge",
        "velocity",
        "bungeecord",
        "geyser",
        "floodgate",
        "server.properties",
        "spigot.yml",
        "paper-world-defaults.yml",
        "spark",
        "timings",
        "rcon",
        "carpet",
        "开服",
        "服务器",
        "插件",
        "模组",
        "世界",
        "生电",
        "空岛",
        "基岩版",
        "服务器日志",
        "服务器崩溃",
        "服务器诊断",
        "服务器审计",
        "服务器取证",
        "插件生态",
        "server logs",
        "server crash",
        "server diagnosis",
        "server audit",
        "plugin ecosystem",
    ]
    .iter()
    .any(|term| lower.contains(term))
}

pub(crate) fn is_minecraft_plugin_request(query: &str) -> bool {
    let lower = query.to_ascii_lowercase();
    if ["browser plugin", "wordpress plugin", "vscode extension"]
        .iter()
        .any(|term| lower.contains(term))
        || query.contains("浏览器插件")
        || query.contains("浏览器扩展")
        || lower.contains("wordpress 插件")
        || lower.contains("chrome 插件")
        || lower.contains("vscode 插件")
    {
        return false;
    }
    query.contains("插件")
        || lower.contains("minecraft plugin")
        || lower.contains("plugin.yml")
        || lower.contains("paper-plugin.yml")
        || [
            "paper plugin",
            "spigot plugin",
            "bukkit plugin",
            "folia plugin",
            "purpur plugin",
            "taboolib",
            "mockbukkit",
        ]
        .iter()
        .any(|term| lower.contains(term))
        || lower.contains(MINECRAFT_PLUGIN_SKILL_ID)
}

pub(crate) fn context_for_request(query: &str) -> String {
    let lower = query.to_ascii_lowercase();
    let mut references = vec![
        ("references/index.md", REFERENCES_INDEX),
        ("references/workflow-and-planning.md", REFERENCES_WORKFLOW),
        ("references/testing-and-delivery.md", REFERENCES_TESTING),
    ];

    if contains_any(
        &lower,
        &[
            "1.12",
            "1.13",
            "1.20",
            "1.21",
            "26.2",
            "folia",
            "spigot",
            "bukkit",
            "跨版本",
            "兼容",
        ],
    ) {
        references.push(("references/platform-and-versions.md", REFERENCES_PLATFORM));
    }

    if contains_any(
        &lower,
        &[
            "vault",
            "luckperms",
            "placeholderapi",
            "attributeplus",
            "playerpoints",
            "经济",
            "权限",
            "依赖",
            "taboolib",
        ],
    ) {
        references.push((
            "references/dependency-selection.md",
            REFERENCES_DEPENDENCIES,
        ));
        references.push(("references/plugin-api-handbook.md", REFERENCES_API));
    }

    if contains_any(
        &lower,
        &[
            "gui",
            "dialog",
            "界面",
            "菜单",
            "背包",
            "箱子",
            "paiui",
            "arcartx",
            "dragoncore",
            "germengine",
        ],
    ) {
        references.push(("references/gui-and-configuration.md", REFERENCES_GUI));
        if lower.contains("dialog") || lower.contains("对话框") {
            references.push(("references/dialog-editor.md", REFERENCES_DIALOG_EDITOR));
        } else {
            references.push(("references/gui-editor.md", REFERENCES_GUI_EDITOR));
        }
    }

    if contains_any(&lower, &["api", "源码", "方法签名", "material", "事件"]) {
        references.push(("references/local-api-cache.md", REFERENCES_LOCAL_API));
    }

    let mut context = String::from(
        "已启用内置 Minecraft 插件开发 Skill。以下内容是当前请求的开发约束和按需参考文档；请遵循其计划、版本、验证和交付要求。\n\n[SKILL.md]\n",
    );
    context.push_str(SKILL_MD);
    for (path, content) in references {
        context.push_str("\n\n[");
        context.push_str(path);
        context.push_str("]\n");
        context.push_str(content);
    }
    context
}

pub(crate) fn server_context_for_request(query: &str) -> String {
    let lower = query.to_ascii_lowercase();
    let mut references = vec![
        ("references/index.md", SERVER_REFERENCES_INDEX),
        ("references/foundations.md", SERVER_REFERENCES_FOUNDATIONS),
    ];

    if contains_any(
        &lower,
        &[
            "plugin",
            "插件",
            "mod",
            "模组",
            "数据库",
            "mysql",
            "mariadb",
            "redis",
            "经济",
            "权限",
            "配置",
            "下载",
        ],
    ) {
        references.push((
            "references/plugins-and-content.md",
            SERVER_REFERENCES_PLUGINS,
        ));
    }

    if contains_any(
        &lower,
        &[
            "world",
            "世界",
            "空岛",
            "rpg",
            "小游戏",
            "pvp",
            "geyser",
            "floodgate",
            "基岩",
            "资源包",
            "数据包",
            "生电",
            "红石",
            "carpet",
        ],
    ) {
        references.push(("references/worlds-and-modes.md", SERVER_REFERENCES_WORLDS));
    }

    if contains_any(
        &lower,
        &[
            "tps", "mspt", "spark", "timings", "性能", "卡顿", "lag", "实体", "区块", "视距",
            "paper", "spigot", "purpur",
        ],
    ) {
        references.push(("references/performance.md", SERVER_REFERENCES_PERFORMANCE));
    }

    if contains_any(
        &lower,
        &[
            "backup",
            "备份",
            "恢复",
            "迁移",
            "升级",
            "回滚",
            "降级",
            "灾难",
            "数据库",
        ],
    ) {
        references.push((
            "references/recovery-and-migration.md",
            SERVER_REFERENCES_RECOVERY,
        ));
    }

    let analysis_request = contains_any(
        &lower,
        &[
            "$manage-minecraft-server",
            "manage-minecraft-server",
            "服务器日志",
            "服务器崩溃",
            "服务器诊断",
            "服务器审计",
            "服务器取证",
            "插件生态",
            "server logs",
            "server crash",
            "server diagnosis",
            "server audit",
            "plugin ecosystem",
        ],
    );

    let mut context = String::from(
        "已启用内置 Minecraft 服务器运维 Skill。以下内容是当前请求的服务器决策约束和按需参考文档；请先保护数据，再按证据执行变更并验证回滚路径。\n\n[SKILL.md]\n",
    );
    context.push_str(SERVER_SKILL_MD);
    for (path, content) in references {
        context.push_str("\n\n[");
        context.push_str(path);
        context.push_str("]\n");
        context.push_str(content);
    }
    if analysis_request {
        context.push_str("\n\n[analysis-SKILL.md]\n");
        context.push_str(SERVER_ANALYSIS_SKILL_MD);
        context.push_str("\n\n[analysis/references/safety.md]\n");
        context.push_str(SERVER_ANALYSIS_SAFETY);
        context.push_str("\n\n[analysis/references/diagnostics.md]\n");
        context.push_str(SERVER_ANALYSIS_DIAGNOSTICS);
        context.push_str("\n\n[analysis/references/plugin-ecosystem.md]\n");
        context.push_str(SERVER_ANALYSIS_PLUGINS);
        context.push_str("\n\n[analysis/references/approval-policy.md]\n");
        context.push_str(SERVER_ANALYSIS_APPROVAL);
    }
    context
}

fn contains_any(value: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| value.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_skill_has_expected_identity() {
        assert!(SKILL_MD.contains("name: develop-minecraft-server-plugin"));
        assert!(SKILL_MD.contains("description:"));
        assert!(REFERENCES_INDEX.contains("workflow-and-planning.md"));
    }

    #[test]
    fn bundled_server_skill_has_expected_identity() {
        assert!(SERVER_SKILL_MD.contains("name: minecraft-server-operations"));
        assert!(SERVER_SKILL_MD.contains("插件生电默认推荐 Leaves"));
        assert!(SERVER_REFERENCES_FOUNDATIONS.contains("插件生电优先选择 Leaves"));
        assert!(SERVER_REFERENCES_INDEX.contains("foundations.md"));
        assert!(SERVER_REFERENCES_RECOVERY.contains("3-2-1"));
    }

    #[test]
    fn plugin_request_detection_covers_explicit_and_common_forms() {
        assert!(is_minecraft_plugin_request("开发一个 Paper 插件"));
        assert!(is_minecraft_plugin_request(
            "create a Minecraft plugin with Vault"
        ));
        assert!(is_minecraft_plugin_request(
            "$develop-minecraft-server-plugin"
        ));
        assert!(!is_minecraft_plugin_request("开发一个浏览器插件"));
        assert!(!is_minecraft_plugin_request(
            "查看服务器 TPS 和玩家在线情况"
        ));
    }

    #[test]
    fn bundled_skill_migration_is_idempotent_and_preserves_disable_state() {
        let mut skills = vec![builtin_skill_info()];
        skills[0].enabled = false;
        assert!(!ensure_bundled_skill(&mut skills));
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].enabled);

        let mut missing = Vec::new();
        assert!(ensure_bundled_skill(&mut missing));
        assert!(!ensure_bundled_skill(&mut missing));
        assert_eq!(missing.len(), 1);
    }

    #[test]
    fn request_context_always_includes_core_workflow_and_selects_api_reference() {
        let context = context_for_request("开发一个 Paper 插件，使用 Vault 和 LuckPerms");
        assert!(context.contains("[SKILL.md]"));
        assert!(context.contains("workflow-and-planning.md"));
        assert!(context.contains("plugin-api-handbook.md"));
        assert!(context.contains("Vault"));
    }

    #[test]
    fn server_request_detection_and_routing_cover_operations_topics() {
        assert!(is_minecraft_server_request("帮我部署一个 Paper 服务器"));
        assert!(is_minecraft_server_request("查看服务器 TPS 和备份状态"));
        assert!(is_minecraft_server_request(
            "我想要和5个朋友一起开一个插件生电服"
        ));
        assert!(!is_minecraft_server_request("写一个普通的 Rust CLI"));
        assert!(!is_minecraft_server_request("开发一个浏览器插件"));

        let context = server_context_for_request("排查 Geyser 基岩版连接和 TPS 卡顿");
        assert!(context.contains("foundations.md"));
        assert!(context.contains("worlds-and-modes.md"));
        assert!(context.contains("performance.md"));
        assert!(!context.contains("## 备份原则"));

        let technical_context = server_context_for_request("我想要和5个朋友一起开一个插件生电服");
        assert!(technical_context.contains("plugins-and-content.md"));
        assert!(technical_context.contains("worlds-and-modes.md"));
    }

    #[test]
    fn merged_server_skill_routes_read_only_analysis_guidance() {
        assert!(is_minecraft_server_request(
            "$manage-minecraft-server 分析服务器日志"
        ));
        let context = server_context_for_request("审计服务器插件生态和崩溃日志");
        assert!(context.contains("analysis-SKILL.md"));
        assert!(context.contains("analysis/references/safety.md"));
        assert!(context.contains("execution_performed=false"));
    }

    #[test]
    fn bundled_server_skill_migration_is_idempotent_and_preserves_disable_state() {
        let mut skills = vec![builtin_server_skill_info()];
        skills[0].enabled = false;
        assert!(!ensure_bundled_server_skill(&mut skills));
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].enabled);

        let mut missing = Vec::new();
        assert!(ensure_bundled_server_skill(&mut missing));
        assert!(!ensure_bundled_server_skill(&mut missing));
        assert_eq!(missing.len(), 1);
    }

    #[test]
    fn bundled_server_skill_contains_intelligent_lookup_protocol() {
        assert!(SERVER_SKILL_MD.contains("res.mcmy.love"));
        assert!(SERVER_SKILL_MD.contains("api.mslmc.cn/v4"));
        assert!(SERVER_SKILL_MD.contains("api.modrinth.com/v2/search"));
        assert!(SERVER_SKILL_MD.contains("www.spigotmc.org/resources"));
        assert!(SERVER_SKILL_MD.contains("QQ/NapCat"));
    }
}
