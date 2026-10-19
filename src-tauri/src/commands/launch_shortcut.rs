use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[derive(Default)]
pub struct LaunchConsoleShortcut(Mutex<Option<Shortcut>>);

fn parse_shortcut(value: &str) -> Result<Option<Shortcut>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let shortcut: Shortcut = value
        .trim()
        .parse()
        .map_err(|error| format!("快捷键无效：{error}"))?;
    if shortcut == crate::show_window_shortcut() {
        return Err("Alt+F 已保留给 SSMT 窗口快捷键，请选择其他组合".into());
    }
    Ok(Some(shortcut))
}

// 先注册新组合再释放旧组合；冲突时保留原快捷键，且只注销本功能拥有的组合。
fn replace_shortcut(
    current: &mut Option<Shortcut>,
    next: Option<Shortcut>,
    mut register: impl FnMut(Shortcut) -> Result<(), String>,
    mut unregister: impl FnMut(Shortcut) -> Result<(), String>,
) -> Result<(), String> {
    if *current == next {
        return Ok(());
    }
    if let Some(shortcut) = next {
        register(shortcut)?;
    }
    if let Some(old) = *current {
        if let Err(error) = unregister(old) {
            if let Some(shortcut) = next {
                let _ = unregister(shortcut);
            }
            return Err(error);
        }
    }
    *current = next;
    Ok(())
}

#[tauri::command]
pub async fn set_launch_console_shortcut(
    app: tauri::AppHandle,
    shortcut: String,
) -> Result<(), String> {
    let next = parse_shortcut(&shortcut)?;
    let state = app.state::<LaunchConsoleShortcut>();
    let mut current = state.0.lock().map_err(|error| error.to_string())?;
    replace_shortcut(
        &mut current,
        next,
        |shortcut| {
            if app.global_shortcut().is_registered(shortcut) {
                return Err("该快捷键已被 SSMT 的其他功能占用".into());
            }
            app.global_shortcut()
                .on_shortcut(shortcut, |app, _, event| {
                    if event.state == ShortcutState::Pressed {
                        let _ = app.emit("open-launch-console", ());
                        crate::reveal_main_window(app);
                    }
                })
                .map_err(|error| format!("无法注册快捷键，可能已被其他程序占用：{error}"))
        },
        |shortcut| {
            app.global_shortcut()
                .unregister(shortcut)
                .map_err(|error| error.to_string())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn supports_disable_and_rejects_invalid_or_reserved_shortcuts() {
        assert_eq!(parse_shortcut(" ").unwrap(), None);
        assert!(parse_shortcut("Ctrl+Shift+L").unwrap().is_some());
        assert!(parse_shortcut("Alt+F").is_err());
        assert!(parse_shortcut("not a shortcut").is_err());
    }

    #[test]
    fn collision_preserves_old_registration_and_same_key_is_idempotent() {
        let old = parse_shortcut("Ctrl+Shift+L").unwrap();
        let next = parse_shortcut("Ctrl+Shift+K").unwrap();
        let mut current = old;
        assert!(replace_shortcut(
            &mut current,
            next,
            |_| Err("occupied".into()),
            |_| panic!("不能注销原快捷键")
        )
        .is_err());
        assert_eq!(current, old);
        replace_shortcut(
            &mut current,
            old,
            |_| panic!("不能重复注册"),
            |_| panic!("不能注销"),
        )
        .unwrap();
        let removed = RefCell::new(Vec::new());
        replace_shortcut(
            &mut current,
            None,
            |_| panic!("禁用不能注册"),
            |key| {
                removed.borrow_mut().push(key);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*removed.borrow(), [old.unwrap()]);
        assert!(current.is_none());
    }

    #[test]
    fn unregister_failure_rolls_back_new_registration() {
        let old = parse_shortcut("Ctrl+Shift+L").unwrap();
        let next = parse_shortcut("Ctrl+Shift+K").unwrap();
        let mut current = old;
        let removed = RefCell::new(Vec::new());
        assert!(replace_shortcut(
            &mut current,
            next,
            |_| Ok(()),
            |key| {
                removed.borrow_mut().push(key);
                if Some(key) == old {
                    Err("unregister failed".into())
                } else {
                    Ok(())
                }
            }
        )
        .is_err());
        assert_eq!(current, old);
        assert_eq!(*removed.borrow(), [old.unwrap(), next.unwrap()]);
    }
}
