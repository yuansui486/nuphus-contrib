//! Workflow external-input contract validation and resolution.

use crate::workflow::types::{InputKind, InputSpec};
use std::collections::{HashMap, HashSet};

/// Names owned by the executor and therefore unavailable to external inputs.
const RESERVED_NAMES: &[&str] = &["inputs", "params", "_index"];

pub fn value_matches_kind(kind: InputKind, value: &serde_json::Value) -> bool {
    match kind {
        InputKind::String | InputKind::Path | InputKind::Enum => value.is_string(),
        InputKind::Number => value.is_number(),
        InputKind::Boolean => value.is_boolean(),
        InputKind::Json => true,
    }
}

fn kind_label(kind: InputKind) -> &'static str {
    match kind {
        InputKind::String => "string",
        InputKind::Number => "number",
        InputKind::Boolean => "boolean",
        InputKind::Path => "path",
        InputKind::Json => "json",
        InputKind::Enum => "enum",
    }
}

/// Validate the declarations themselves. Errors are stable, user-facing strings.
pub fn validate_specs(specs: &[InputSpec]) -> Vec<String> {
    let name_re = regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$")
        .expect("input name regex is statically valid");
    let mut errors = Vec::new();
    let mut seen = HashSet::new();

    for spec in specs {
        if !name_re.is_match(&spec.name) {
            errors.push(format!(
                "输入声明名称 '{}' 非法（必须匹配 [A-Za-z_][A-Za-z0-9_]*）",
                spec.name
            ));
        }
        if RESERVED_NAMES.contains(&spec.name.as_str()) {
            errors.push(format!("输入声明名称 '{}' 是保留名", spec.name));
        }
        if !seen.insert(spec.name.as_str()) {
            errors.push(format!("重复的输入声明名称: '{}'", spec.name));
        }
        if let Some(default) = &spec.default {
            if !value_matches_kind(spec.kind, default) {
                errors.push(format!(
                    "输入 '{}': 默认值类型必须是 {}",
                    spec.name,
                    kind_label(spec.kind)
                ));
            }
        }
        // enum 专项：options 必填非空、不含空档位；default 必须命中白名单
        if spec.kind.is_enum() {
            let empty = spec.options.as_ref().is_none_or(|options| {
                options.is_empty() || options.iter().any(|o| o.trim().is_empty())
            });
            if empty {
                errors.push(format!(
                    "输入 '{}': type=enum 必须提供非空 options 档位列表",
                    spec.name
                ));
            } else if let Some(default) = &spec.default {
                let default_str = default.as_str().unwrap_or_default().to_string();
                let in_options = spec
                    .options
                    .as_ref()
                    .is_some_and(|options| options.contains(&default_str));
                if !in_options {
                    errors.push(format!(
                        "输入 '{}': 默认值不在 options 档位列表内",
                        spec.name
                    ));
                }
            }
        } else if spec.options.is_some() {
            // 非 enum 类型带 options：静默忽略语义易误导，直接判为声明错误
            errors.push(format!(
                "输入 '{}': 仅 type=enum 支持 options 字段",
                spec.name
            ));
        }
    }
    errors
}

/// Resolve declared inputs using explicit value > declaration default > absent optional.
/// Undeclared values are deliberately ignored here; callers that preserve legacy top-level
/// injection can still use the original `provided` map after this function succeeds.
pub fn resolve_declared_inputs(
    specs: &[InputSpec],
    provided: &HashMap<String, serde_json::Value>,
) -> crate::Result<serde_json::Map<String, serde_json::Value>> {
    let mut resolved = serde_json::Map::new();
    for spec in specs {
        let value = provided.get(&spec.name).or(spec.default.as_ref());
        match value {
            Some(value) if value_matches_kind(spec.kind, value) => {
                // enum 白名单终检：显式值与 default 都必须 ∈ options（default 已在声明校验挡过，
                // 此处兜底运行时传入值，两处统一报错口径）
                if spec.kind.is_enum() {
                    let allowed = spec.options.as_deref().unwrap_or_default();
                    let actual = value.as_str().unwrap_or_default();
                    if !allowed.iter().any(|option| option == actual) {
                        return Err(crate::NuphusError::agent(format!(
                            "输入 '{}': 值 '{}' 不在档位列表 [{}]",
                            spec.name,
                            actual,
                            allowed.join(", ")
                        )));
                    }
                }
                resolved.insert(spec.name.clone(), value.clone());
            }
            Some(_) => {
                return Err(crate::NuphusError::agent(format!(
                    "输入 '{}': 值类型必须是 {}",
                    spec.name,
                    kind_label(spec.kind)
                )));
            }
            None if spec.required => {
                return Err(crate::NuphusError::agent(format!(
                    "缺少必填输入：{}",
                    spec.name
                )));
            }
            None => {}
        }
    }
    Ok(resolved)
}

/// Reject keys not present in the target workflow declaration, then resolve the contract.
/// Used by persisted bindings such as schedules where compatibility-only ad-hoc keys are unsafe.
pub fn resolve_declared_only(
    specs: &[InputSpec],
    provided: &HashMap<String, serde_json::Value>,
) -> crate::Result<serde_json::Map<String, serde_json::Value>> {
    let declared: HashSet<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
    if let Some(name) = provided
        .keys()
        .find(|name| !declared.contains(name.as_str()))
    {
        return Err(crate::NuphusError::agent(format!(
            "输入 '{}' 未在目标工作流中声明",
            name
        )));
    }
    resolve_declared_inputs(specs, provided)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, kind: InputKind) -> InputSpec {
        InputSpec {
            name: name.into(),
            kind,
            required: false,
            default: None,
            description: None,
            sensitive: false,
            options: None,
        }
    }

    #[test]
    fn declaration_names_duplicates_reserved_and_defaults_are_validated() {
        let mut bad_default = spec("count", InputKind::Number);
        bad_default.default = Some(serde_json::json!("one"));
        let errors = validate_specs(&[
            spec("bad-name", InputKind::String),
            spec("inputs", InputKind::Json),
            spec("same", InputKind::String),
            spec("same", InputKind::String),
            bad_default,
        ]);
        assert_eq!(errors.len(), 4, "{errors:?}");
    }

    #[test]
    fn enum_specs_require_options_and_whitelist_defaults_and_values() {
        // ① 无 options → 声明错误
        let errors = validate_specs(&[spec("mode", InputKind::Enum)]);
        assert!(errors.iter().any(|e| e.contains("options")), "{errors:?}");

        // ② default 不在白名单 → 声明错误
        let mut bad = spec("mode", InputKind::Enum);
        bad.options = Some(vec!["普通".into(), "后台".into()]);
        bad.default = Some(serde_json::json!("硬件"));
        let errors = validate_specs(&[bad]);
        assert!(
            errors.iter().any(|e| e.contains("不在 options")),
            "{errors:?}"
        );

        // ③ 非 enum 类型带 options → 声明错误
        let mut stray = spec("text", InputKind::String);
        stray.options = Some(vec!["x".into()]);
        let errors = validate_specs(&[stray]);
        assert!(
            errors.iter().any(|e| e.contains("仅 type=enum")),
            "{errors:?}"
        );

        // ④ 运行时：合法档位通过；非法档位报白名单错误
        let mut mode = spec("mode", InputKind::Enum);
        mode.options = Some(vec!["普通".into(), "后台".into()]);
        let ok = resolve_declared_inputs(
            &[mode.clone()],
            &HashMap::from([("mode".into(), serde_json::json!("后台"))]),
        )
        .unwrap();
        assert_eq!(ok["mode"], serde_json::json!("后台"));

        let err = resolve_declared_inputs(
            &[mode],
            &HashMap::from([("mode".into(), serde_json::json!("驱动级"))]),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("不在档位列表"), "{err}");

        // ⑤ default 兜底路径同样过白名单（resolve 用 default 时）
        let mut with_default = spec("mode", InputKind::Enum);
        with_default.options = Some(vec!["a".into(), "b".into()]);
        with_default.default = Some(serde_json::json!("b"));
        let resolved = resolve_declared_inputs(&[with_default], &HashMap::new()).unwrap();
        assert_eq!(resolved["mode"], serde_json::json!("b"));
    }

    #[test]
    fn runtime_types_and_required_values_are_checked() {
        let mut required = spec("enabled", InputKind::Boolean);
        required.required = true;
        let err = resolve_declared_inputs(&[required.clone()], &HashMap::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("缺少必填输入"));

        let provided = HashMap::from([("enabled".into(), serde_json::json!("yes"))]);
        let err = resolve_declared_inputs(&[required], &provided)
            .unwrap_err()
            .to_string();
        assert!(err.contains("boolean"));
    }

    #[test]
    fn undeclared_values_are_compatible_for_runs_but_rejected_for_bindings() {
        let provided = HashMap::from([("legacy".into(), serde_json::json!(1))]);
        assert!(resolve_declared_inputs(&[], &provided).unwrap().is_empty());
        assert!(resolve_declared_only(&[], &provided).is_err());
    }
}
