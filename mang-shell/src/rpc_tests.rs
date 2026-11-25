//! #47 巨石拆解批⑤：原 rpc.rs 内嵌 `tests` 纯迁出
//!（零行为变化——字节原样剥壳，#[cfg(test)] + #[path] 挂载在宿主文件）。

use super::*;

// ---------------- #111 G1 写回半：read_file 载荷解码 ----------------

#[test]
fn read_payload_decodes_gbk_utf16_bom() {
    // GBK / UTF-16LE（BOM）/ UTF-8 BOM 文档 browse 面可读（先红：
    // 旧实现 from_utf8 直拒——p4tr §6 双读路径备忘的缺口）。
    let gbk: &[u8] = &[0xD6, 0xD0, 0xCE, 0xC4, 0xCE, 0xC4, 0xB5, 0xB5];
    assert_eq!(
        decode_read_payload(gbk, 1024).unwrap(),
        ("中文文档".to_string(), false)
    );
    let u16le: &[u8] = &[0xFF, 0xFE, 0x2D, 0x4E, 0x87, 0x65];
    assert_eq!(
        decode_read_payload(u16le, 1024).unwrap(),
        ("中文".to_string(), false)
    );
    // BOM 剥离（旧实现把 U+FEFF 留在 text 首位——新实现与打开链
    // 同口径）。
    let bom_utf8: &[u8] = &[0xEF, 0xBB, 0xBF, b'a', b'b'];
    assert_eq!(
        decode_read_payload(bom_utf8, 1024).unwrap(),
        ("ab".to_string(), false)
    );
    // 纯 UTF-8 直通；二进制/未知编码拒绝不猜。
    assert_eq!(
        decode_read_payload(b"plain", 1024).unwrap(),
        ("plain".to_string(), false)
    );
    assert!(decode_read_payload(&[0x81, 0x40, 0xFF], 1024).is_err());
}

#[test]
fn read_payload_truncates_on_decoded_char_boundary() {
    // 截断落在解码后 UTF-8 字节面（载荷按字节计）且不劈多字节字符。
    // 「中文ab」解码后 = 3+3+1+1 = 8 字节：cap=6 恰好落在 a 的起点
    // → 「中文」；cap=5 落在「文」末字节 → 退让到 3（「文」起点）→
    // 「中」。cap 在此按解码文本计（UTF-16 源 10 字节也截同一处）。
    let text = "中文ab";
    let bytes: Vec<u8> = {
        let mut v = vec![0xFF, 0xFE]; // UTF-16LE 装载同一段文本
        for unit in text.encode_utf16() {
            v.extend_from_slice(&unit.to_le_bytes());
        }
        v
    };
    let (out, truncated) = decode_read_payload(&bytes, 6).unwrap();
    assert!(truncated, "解码文本超 cap 应标记截断");
    assert_eq!(out, "中文", "cap=6 落在字符边界——完整保留两个汉字");
    let (out5, _) = decode_read_payload(&bytes, 5).unwrap();
    assert_eq!(out5, "中", "cap=5 劈在「文」中——退让到字符起点");
}

// ---------------- 协议解析 ----------------

#[test]
fn parse_six_commands_roundtrip() {
    let (id, cmd) = parse_command(r#"{"id":1,"cmd":"open_file","path":"/tmp/a.md"}"#);
    assert_eq!(id, Some(1));
    assert_eq!(
        cmd,
        Ok(ControlCommand::OpenFile {
            path: "/tmp/a.md".into()
        })
    );

    let (id, cmd) = parse_command(r#"{"id":2,"cmd":"insert_text","line":3,"col":4,"text":"hi\n"}"#);
    assert_eq!(id, Some(2));
    assert_eq!(
        cmd,
        Ok(ControlCommand::InsertText {
            line: 3,
            col: 4,
            text: "hi\n".into()
        })
    );

    let (id, cmd) = parse_command(r#"{"id":3,"cmd":"replace_selection","text":"X"}"#);
    assert_eq!(id, Some(3));
    assert_eq!(
        cmd,
        Ok(ControlCommand::ReplaceSelection { text: "X".into() })
    );

    let (id, cmd) = parse_command(r#"{"id":4,"cmd":"get_state"}"#);
    assert_eq!(id, Some(4));
    assert_eq!(cmd, Ok(ControlCommand::GetState));

    let (id, cmd) = parse_command(r#"{"id":5,"cmd":"close_file","path":"/tmp/a.md"}"#);
    assert_eq!(id, Some(5));
    assert_eq!(
        cmd,
        Ok(ControlCommand::CloseFile {
            path: "/tmp/a.md".into()
        })
    );

    let (id, cmd) = parse_command(r#"{"id":6,"cmd":"switch_tab","index":1}"#);
    assert_eq!(id, Some(6));
    assert_eq!(cmd, Ok(ControlCommand::SwitchTab { index: 1 }));
}

#[test]
fn parse_id_optional_and_extra_fields_ignored() {
    // 无 id → None（响应回 null）
    let (id, cmd) = parse_command(r#"{"cmd":"get_state"}"#);
    assert_eq!(id, None);
    assert_eq!(cmd, Ok(ControlCommand::GetState));

    // 未知字段忽略（前向兼容——新客户端给旧服务发附加键不炸）
    let (id, cmd) = parse_command(r#"{"id":9,"cmd":"get_state","future_field":true}"#);
    assert_eq!(id, Some(9));
    assert_eq!(cmd, Ok(ControlCommand::GetState));
}

#[test]
fn parse_malformed_json_yields_null_id_and_error() {
    let (id, cmd) = parse_command("not json at all");
    assert_eq!(id, None);
    assert!(cmd.is_err());

    let (id, cmd) = parse_command(r#"{"id":7,"cmd":"get_state""#); // 截断
    assert_eq!(id, None, "截断 JSON 取不到 id——回 null");
    assert!(cmd.is_err());
}

#[test]
fn parse_unknown_command_echoes_id() {
    let (id, cmd) = parse_command(r#"{"id":42,"cmd":"open_fiel","path":"/x"}"#);
    assert_eq!(id, Some(42), "JSON 合法但命令拼错——id 应回显");
    let err = cmd.unwrap_err();
    assert!(err.contains("open_fiel"), "错误消息含未知命令名：{err}");
}

#[test]
fn parse_missing_or_wrong_typed_fields_error() {
    // 缺 cmd
    let (id, cmd) = parse_command(r#"{"id":1,"path":"/x"}"#);
    assert_eq!(id, Some(1));
    assert!(cmd.is_err());

    // insert_text 缺 text
    let (_, cmd) = parse_command(r#"{"id":2,"cmd":"insert_text","line":0,"col":0}"#);
    assert!(cmd.is_err());

    // line 类型错（字符串）
    let (_, cmd) = parse_command(r#"{"id":3,"cmd":"insert_text","line":"0","col":0,"text":"x"}"#);
    assert!(cmd.is_err());

    // switch_tab 缺 index
    let (_, cmd) = parse_command(r#"{"id":4,"cmd":"switch_tab"}"#);
    assert!(cmd.is_err());
}

#[test]
fn response_serialization_shapes() {
    let ok = ControlResponse::ok(Some(5), serde_json::json!({"n": 1}));
    let s = ok.to_line();
    assert!(s.ends_with('\n'), "行协议以换行收尾");
    let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
    assert_eq!(v["id"], 5);
    assert_eq!(v["ok"], true);
    assert_eq!(v["result"]["n"], 1);
    assert!(v.get("error").is_none(), "ok 分支不出现 error 键");

    let err = ControlResponse::err(None, "炸了".into());
    let v: serde_json::Value = serde_json::from_str(err.to_line().trim()).unwrap();
    assert_eq!(v["id"], serde_json::Value::Null);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"], "炸了");
    assert!(v.get("result").is_none(), "err 分支不出现 result 键");
}

#[test]
fn command_names_match_wire_tags() {
    // 证据行/日志面与线上 cmd 字段一致性——防止改名漂移
    assert_eq!(
        ControlCommand::OpenFile {
            path: String::new()
        }
        .name(),
        "open_file"
    );
    assert_eq!(
        ControlCommand::InsertText {
            line: 0,
            col: 0,
            text: String::new()
        }
        .name(),
        "insert_text"
    );
    assert_eq!(
        ControlCommand::ReplaceSelection {
            text: String::new()
        }
        .name(),
        "replace_selection"
    );
    assert_eq!(ControlCommand::GetState.name(), "get_state");
    assert_eq!(
        ControlCommand::CloseFile {
            path: String::new()
        }
        .name(),
        "close_file"
    );
    assert_eq!(ControlCommand::SwitchTab { index: 0 }.name(), "switch_tab");
}

// ---------------- #111：新命令解析（8 命令 + 订阅） ----------------

#[test]
fn parse_eight_new_commands_roundtrip() {
    let (id, cmd) = parse_command(r#"{"id":7,"cmd":"set_cursor","line":10,"col":5}"#);
    assert_eq!(id, Some(7));
    assert_eq!(cmd, Ok(ControlCommand::SetCursor { line: 10, col: 5 }));

    let (id, cmd) = parse_command(
        r#"{"id":8,"cmd":"set_selection","start":{"line":0,"col":1},"end":{"line":0,"col":5}}"#,
    );
    assert_eq!(id, Some(8));
    assert_eq!(
        cmd,
        Ok(ControlCommand::SetSelection {
            start: PointWire { line: 0, col: 1 },
            end: PointWire { line: 0, col: 5 },
        })
    );

    let (_, cmd) = parse_command(r#"{"id":9,"cmd":"get_selection"}"#);
    assert_eq!(cmd, Ok(ControlCommand::GetSelection));

    let (_, cmd) = parse_command(r#"{"id":10,"cmd":"undo"}"#);
    assert_eq!(cmd, Ok(ControlCommand::Undo));

    let (_, cmd) = parse_command(r#"{"id":11,"cmd":"redo"}"#);
    assert_eq!(cmd, Ok(ControlCommand::Redo));

    let (_, cmd) = parse_command(r#"{"id":12,"cmd":"save"}"#);
    assert_eq!(cmd, Ok(ControlCommand::Save));

    let (_, cmd) = parse_command(r#"{"id":13,"cmd":"save_as","path":"/tmp/b.md"}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::SaveAs {
            path: "/tmp/b.md".into()
        })
    );

    let (_, cmd) = parse_command(r#"{"id":14,"cmd":"scroll_to","line":42}"#);
    assert_eq!(cmd, Ok(ControlCommand::ScrollTo { line: 42 }));

    let (id, cmd) = parse_command(r#"{"id":15,"cmd":"subscribe"}"#);
    assert_eq!(id, Some(15));
    assert_eq!(cmd, Ok(ControlCommand::Subscribe));
    assert_eq!(cmd.unwrap().name(), "subscribe");
}

#[test]
fn parse_new_commands_field_errors() {
    // set_cursor 缺 col
    let (_, cmd) = parse_command(r#"{"cmd":"set_cursor","line":1}"#);
    assert!(cmd.is_err());

    // set_selection 的点位也接受数组形态（serde struct 的 seq 形式
    // ——[line, col] 按位置解释；issue 事件示例即数组口径，两种
    // 写法互通是宽容输入面）。
    let (_, cmd) =
        parse_command(r#"{"cmd":"set_selection","start":[0,1],"end":{"line":0,"col":5}}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::SetSelection {
            start: PointWire { line: 0, col: 1 },
            end: PointWire { line: 0, col: 5 },
        }),
        "数组点位按 [line, col] 位置解释——宽容输入"
    );

    // save_as 缺 path
    let (_, cmd) = parse_command(r#"{"cmd":"save_as"}"#);
    assert!(cmd.is_err());

    // scroll_to 的 line 类型错（字符串）
    let (_, cmd) = parse_command(r#"{"cmd":"scroll_to","line":"42"}"#);
    assert!(cmd.is_err());
}

#[test]
fn new_command_names_match_wire_tags() {
    assert_eq!(
        ControlCommand::SetCursor { line: 0, col: 0 }.name(),
        "set_cursor"
    );
    assert_eq!(
        ControlCommand::SetSelection {
            start: PointWire { line: 0, col: 0 },
            end: PointWire { line: 0, col: 0 },
        }
        .name(),
        "set_selection"
    );
    assert_eq!(ControlCommand::GetSelection.name(), "get_selection");
    assert_eq!(ControlCommand::Undo.name(), "undo");
    assert_eq!(ControlCommand::Redo.name(), "redo");
    assert_eq!(ControlCommand::Save.name(), "save");
    assert_eq!(
        ControlCommand::SaveAs {
            path: String::new()
        }
        .name(),
        "save_as"
    );
    assert_eq!(ControlCommand::ScrollTo { line: 0 }.name(), "scroll_to");
}

// ---------------- #111：订阅表逻辑 ----------------

#[test]
fn subscribe_insert_is_idempotent_and_broadcast_reaches_all() {
    let subs: Subscriptions = std::sync::Mutex::new(std::collections::HashMap::new());
    let (a_tx, a_rx) = async_channel::unbounded::<String>();
    let (b_tx, b_rx) = async_channel::unbounded::<String>();
    subscribe_insert(&subs, 1, a_tx);
    subscribe_insert(&subs, 2, b_tx);
    // 幂等：同 conn 重新订阅替换旧通道（旧 rx 不再收）
    let (a2_tx, a2_rx) = async_channel::unbounded::<String>();
    subscribe_insert(&subs, 1, a2_tx);
    assert_eq!(subs.lock().unwrap().len(), 2, "同 conn 重复订阅不增条目");

    let delivered = subscribe_broadcast(&subs, "event\n");
    assert_eq!(delivered, 2, "两路存活订阅均送达");
    assert!(a_rx.is_empty(), "旧通道被替换——不再投递");
    assert_eq!(
        a2_rx.recv_blocking().as_deref(),
        Ok("event\n"),
        "重新订阅的新通道收到"
    );
    assert_eq!(
        b_rx.recv_blocking().as_deref(),
        Ok("event\n"),
        "未受重订阅影响的另一路照常收到"
    );
}

#[test]
fn subscribe_broadcast_drops_dead_subscribers() {
    let subs: Subscriptions = std::sync::Mutex::new(std::collections::HashMap::new());
    let (tx, rx) = async_channel::unbounded::<String>();
    // 接收端立即 drop——通道已死
    drop(rx);
    subscribe_insert(&subs, 7, tx);
    let delivered = subscribe_broadcast(&subs, "line\n");
    assert_eq!(delivered, 0, "死通道不计送达");
    assert!(subs.lock().unwrap().is_empty(), "广播后死订阅被自动移除");
}

#[test]
fn doc_event_line_serialization_for_subscribers() {
    // 事件 → JSON 行（事件泵路径的形态钉定：以 \n 收尾、type 字段名）
    let ev = crate::doc_events::DocEvent {
        kind: "doc_changed",
        path: Some(PathBuf::from("/tmp/a.md")),
        revision_before: 0,
        revision: 1,
        source: "rpc",
        changes: vec![crate::doc_events::ChangeRecord::precise(
            "replace",
            Point::new(0, 0),
            Point::new(0, 0),
            "hi".into(),
        )],
    };
    let mut line = serde_json::to_string(&ev).unwrap();
    line.push('\n');
    let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(parsed["type"], "doc_changed");
    assert_eq!(parsed["source"], "rpc");
    assert_eq!(parsed["changes"][0]["kind"], "replace");
}

// ---------------- #134 任务 1：FS 命令协议 + 沙箱 ----------------

#[test]
fn parse_fs_commands_roundtrip() {
    // list_dir：recursive 缺省 false（serde default——宽容输入）。
    let (id, cmd) = parse_command(r#"{"id":16,"cmd":"list_dir","path":"src"}"#);
    assert_eq!(id, Some(16));
    assert_eq!(
        cmd,
        Ok(ControlCommand::ListDir {
            path: "src".into(),
            recursive: false
        })
    );

    let (_, cmd) = parse_command(r#"{"id":17,"cmd":"list_dir","path":".","recursive":true}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::ListDir {
            path: ".".into(),
            recursive: true
        })
    );

    let (_, cmd) = parse_command(r#"{"id":18,"cmd":"read_file","path":"a.rs"}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::ReadFile {
            path: "a.rs".into()
        })
    );

    let (_, cmd) =
        parse_command(r#"{"id":19,"cmd":"write_file","path":"a.rs","content":"fn main(){}"}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::WriteFile {
            path: "a.rs".into(),
            content: "fn main(){}".into()
        })
    );

    // create_file：content 可省（None = 空文件）。
    let (_, cmd) = parse_command(r#"{"id":20,"cmd":"create_file","path":"b.rs"}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::CreateFile {
            path: "b.rs".into(),
            content: None
        })
    );

    let (_, cmd) = parse_command(r#"{"id":21,"cmd":"create_file","path":"b.rs","content":"x"}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::CreateFile {
            path: "b.rs".into(),
            content: Some("x".into())
        })
    );

    let (_, cmd) = parse_command(r#"{"id":22,"cmd":"create_dir","path":"src/sub"}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::CreateDir {
            path: "src/sub".into()
        })
    );

    let (_, cmd) = parse_command(r#"{"id":23,"cmd":"delete","path":"old.rs"}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::Delete {
            path: "old.rs".into()
        })
    );

    let (_, cmd) = parse_command(r#"{"id":24,"cmd":"rename","from":"a.rs","to":"b.rs"}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::Rename {
            from: "a.rs".into(),
            to: "b.rs".into()
        })
    );
}

#[test]
fn parse_fs_commands_field_errors() {
    // read_file 缺 path
    let (_, cmd) = parse_command(r#"{"cmd":"read_file"}"#);
    assert!(cmd.is_err());
    // write_file 缺 content
    let (_, cmd) = parse_command(r#"{"cmd":"write_file","path":"x"}"#);
    assert!(cmd.is_err());
    // rename 缺 to
    let (_, cmd) = parse_command(r#"{"cmd":"rename","from":"x"}"#);
    assert!(cmd.is_err());
    // list_dir 的 recursive 类型错（字符串）
    let (_, cmd) = parse_command(r#"{"cmd":"list_dir","path":".","recursive":"yes"}"#);
    assert!(cmd.is_err());
}

#[test]
fn fs_command_names_match_wire_tags() {
    assert_eq!(
        ControlCommand::ListDir {
            path: String::new(),
            recursive: false
        }
        .name(),
        "list_dir"
    );
    assert_eq!(
        ControlCommand::ReadFile {
            path: String::new()
        }
        .name(),
        "read_file"
    );
    assert_eq!(
        ControlCommand::WriteFile {
            path: String::new(),
            content: String::new()
        }
        .name(),
        "write_file"
    );
    assert_eq!(
        ControlCommand::CreateFile {
            path: String::new(),
            content: None
        }
        .name(),
        "create_file"
    );
    assert_eq!(
        ControlCommand::CreateDir {
            path: String::new()
        }
        .name(),
        "create_dir"
    );
    assert_eq!(
        ControlCommand::Delete {
            path: String::new()
        }
        .name(),
        "delete"
    );
    assert_eq!(
        ControlCommand::Rename {
            from: String::new(),
            to: String::new()
        }
        .name(),
        "rename"
    );
}

#[test]
fn parse_editor_command_commands_roundtrip() {
    // run_command：args 可省（前向兼容字段）
    let (id, cmd) = parse_command(r#"{"id":25,"cmd":"run_command","name":"goto_line"}"#);
    assert_eq!(id, Some(25));
    assert_eq!(
        cmd,
        Ok(ControlCommand::RunCommand {
            name: "goto_line".into(),
            args: None,
        })
    );
    // args 显式传（当前忽略——前向兼容）
    let (_, cmd) = parse_command(r#"{"cmd":"run_command","name":"undo","args":{"n":1}}"#);
    assert_eq!(
        cmd,
        Ok(ControlCommand::RunCommand {
            name: "undo".into(),
            args: Some(serde_json::json!({"n": 1})),
        })
    );

    let (_, cmd) = parse_command(r#"{"id":26,"cmd":"list_commands"}"#);
    assert_eq!(cmd, Ok(ControlCommand::ListCommands));

    let (_, cmd) = parse_command(r#"{"id":27,"cmd":"get_outline"}"#);
    assert_eq!(cmd, Ok(ControlCommand::GetOutline));

    // apply_format：点位对象/数组两形态 + 全部格式枚举值
    let (_, cmd) = parse_command(
        r#"{"cmd":"apply_format","range":[{"line":0,"col":0},{"line":0,"col":4}],"format":"bold"}"#,
    );
    assert_eq!(
        cmd,
        Ok(ControlCommand::ApplyFormat {
            range: (PointWire { line: 0, col: 0 }, PointWire { line: 0, col: 4 },),
            format: crate::editor::FormatKind::Bold,
        })
    );
    for (wire, kind) in [
        ("bold", crate::editor::FormatKind::Bold),
        ("italic", crate::editor::FormatKind::Italic),
        ("code", crate::editor::FormatKind::Code),
        ("strikethrough", crate::editor::FormatKind::Strikethrough),
    ] {
        let (_, cmd) = parse_command(&format!(
            r#"{{"cmd":"apply_format","range":[[0,0],[0,1]],"format":"{wire}"}}"#
        ));
        assert_eq!(
            cmd,
            Ok(ControlCommand::ApplyFormat {
                range: (PointWire { line: 0, col: 0 }, PointWire { line: 0, col: 1 },),
                format: kind,
            }),
            "{wire} 应解析到对应格式"
        );
    }
}

#[test]
fn parse_editor_command_commands_field_errors() {
    // run_command 缺 name
    let (_, cmd) = parse_command(r#"{"cmd":"run_command"}"#);
    assert!(cmd.is_err());
    // apply_format 缺 format / range 类型错 / 未知格式值
    let (_, cmd) = parse_command(r#"{"cmd":"apply_format","range":[[0,0],[0,1]]}"#);
    assert!(cmd.is_err());
    let (_, cmd) = parse_command(r#"{"cmd":"apply_format","range":"0,0-0,1","format":"bold"}"#);
    assert!(cmd.is_err());
    let (_, cmd) =
        parse_command(r#"{"cmd":"apply_format","range":[[0,0],[0,1]],"format":"underline"}"#);
    assert!(cmd.is_err(), "未知格式值拒绝");
}

#[test]
fn editor_command_names_match_wire_tags() {
    assert_eq!(
        ControlCommand::RunCommand {
            name: String::new(),
            args: None,
        }
        .name(),
        "run_command"
    );
    assert_eq!(ControlCommand::ListCommands.name(), "list_commands");
    assert_eq!(ControlCommand::GetOutline.name(), "get_outline");
    assert_eq!(
        ControlCommand::ApplyFormat {
            range: (PointWire { line: 0, col: 0 }, PointWire { line: 0, col: 0 },),
            format: crate::editor::FormatKind::Bold,
        }
        .name(),
        "apply_format"
    );
}

#[test]
fn parse_batch_replace_roundtrip() {
    let (id, cmd) = parse_command(
        r#"{"id":30,"cmd":"batch_replace","files":["a.rs","b.rs"],"pattern":"foo","replacement":"bar"}"#,
    );
    assert_eq!(id, Some(30));
    assert_eq!(
        cmd,
        Ok(ControlCommand::BatchReplace {
            files: vec!["a.rs".into(), "b.rs".into()],
            pattern: "foo".into(),
            replacement: "bar".into(),
            is_regex: false,
        })
    );
    // is_regex 显式 true + 空 files 数组形态合法（cmd 层拒）
    let (_, cmd) = parse_command(
        r#"{"cmd":"batch_replace","files":[],"pattern":"f(\\w+)","replacement":"x$1","is_regex":true}"#,
    );
    assert_eq!(
        cmd,
        Ok(ControlCommand::BatchReplace {
            files: vec![],
            pattern: "f(\\w+)".into(),
            replacement: "x$1".into(),
            is_regex: true,
        })
    );
    // 字段错误：缺 pattern / files 类型错
    let (_, cmd) = parse_command(r#"{"cmd":"batch_replace","files":["a"],"replacement":"b"}"#);
    assert!(cmd.is_err());
    let (_, cmd) =
        parse_command(r#"{"cmd":"batch_replace","files":"a","pattern":"x","replacement":"y"}"#);
    assert!(cmd.is_err());
    assert_eq!(
        ControlCommand::BatchReplace {
            files: vec![],
            pattern: String::new(),
            replacement: String::new(),
            is_regex: false,
        }
        .name(),
        "batch_replace"
    );
}

// ---------------- #134 任务 3：collect 层（单测主战场） ----------------

#[test]
fn literal_hits_multiline_and_cjk_cols() {
    let lines = vec![
        "foo and foo".to_string(),
        "中文 foo 混排".to_string(),
        "无命中".to_string(),
    ];
    let hits = collect_literal_hits(&lines, "foo", "X").unwrap();
    assert_eq!(hits.len(), 3, "{hits:?}");
    // 英文行两个命中：col 0 / col 8；非重叠推进
    assert_eq!(
        hits[0],
        ReplaceHit {
            line: 0,
            col: 0,
            len: 3,
            repl: "X".into()
        }
    );
    assert_eq!(
        hits[1],
        ReplaceHit {
            line: 0,
            col: 8,
            len: 3,
            repl: "X".into()
        }
    );
    // CJK 行："foo" 前是 2 汉字 + 1 空格 = 3 chars → char 列 3
    //（字节偏移 7——char 列口径与 Buffer 一致，中文每字 1 列）
    assert_eq!(hits[2].line, 1);
    assert_eq!(hits[2].col, 3, "char 列口径：{:?}", hits[2]);
    assert_eq!(hits[2].len, 3);
}

#[test]
fn literal_hits_non_overlapping_and_empty_pattern() {
    // "aaa" 查 "aa"：只命中 col 0（替换推进后剩余不足）
    let lines = vec!["aaa".to_string()];
    let hits = collect_literal_hits(&lines, "aa", "").unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    // 空模式拒绝
    assert!(collect_literal_hits(&lines, "", "x").is_err());
}

#[test]
fn regex_hits_groups_and_empty_match_skip() {
    let lines = vec!["fn foo() {}".to_string(), "fn bar() {}".to_string()];
    let re = regex::Regex::new("fn (\\w+)").unwrap();
    // 组引用后跟字母时必须用 ${1} 花括号界定（$1_impl 会被解析为
    // 命名组“1_impl”→ 不存在 → 空展开）
    let hits = collect_regex_hits(&lines, &re, "fn ${1}_impl");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].repl, "fn foo_impl", "$1 捕获展开");
    assert_eq!(hits[1].repl, "fn bar_impl");
    assert_eq!(hits[0].col, 0);
    assert_eq!(hits[0].len, 6, "fn foo = 6 chars");

    // 空匹配跳过：a* 在 "b" 上每处空命中——不产生编辑
    let lines = vec!["b".to_string()];
    let re = regex::Regex::new("a*").unwrap();
    let hits = collect_regex_hits(&lines, &re, "X");
    assert!(hits.is_empty(), "空命中不产生编辑：{hits:?}");
}

#[test]
fn replace_text_lines_in_place() {
    let mut lines = vec!["foo x foo".to_string(), "tail".to_string()];
    let hits = collect_literal_hits(&lines, "foo", "BAR").unwrap();
    replace_text_lines(&hits, &mut lines);
    assert_eq!(lines[0], "BAR x BAR");
    assert_eq!(lines[1], "tail", "无命中行不动");

    // CJK 替换（char 域正确性）
    let mut lines = vec!["中文foo混排".to_string()];
    let hits = collect_literal_hits(&lines, "foo", "（替换）").unwrap();
    replace_text_lines(&hits, &mut lines);
    assert_eq!(lines[0], "中文（替换）混排");

    // 正则 $1 展开后的就地替换
    let mut lines = vec!["price: 100; price: 250".to_string()];
    let re = regex::Regex::new("price: (\\d+)").unwrap();
    let hits = collect_regex_hits(&lines, &re, "cost: $1");
    replace_text_lines(&hits, &mut lines);
    assert_eq!(lines[0], "cost: 100; cost: 250");
}

/// 沙箱测试夹具：进程唯一临时工作区（G3 全局污染教训——不共享
/// 可变全局路径；每个用例独立目录）。
fn temp_ws(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mang-rpc-fs-ws-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn sandbox_accepts_inside_paths() {
    let ws = temp_ws("inside");
    std::fs::write(ws.join("a.md"), "x").unwrap();
    std::fs::create_dir(ws.join("src")).unwrap();
    let canon_ws = ws.canonicalize().unwrap();

    // 相对路径 → 相对根解析（AI 主用法）
    assert_eq!(
        resolve_sandboxed(&ws, Path::new("a.md")).unwrap(),
        canon_ws.join("a.md")
    );
    // 根内绝对路径
    assert_eq!(
        resolve_sandboxed(&ws, &ws.join("src")).unwrap(),
        canon_ws.join("src")
    );
    // 根自身（list_dir "." 的合法场景）
    assert_eq!(resolve_sandboxed(&ws, Path::new(".")).unwrap(), canon_ws);
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn sandbox_rejects_outside_paths() {
    let ws = temp_ws("outside");
    // 绝对路径根外
    assert!(
        resolve_sandboxed(&ws, Path::new("/etc/passwd")).is_err(),
        "根外绝对路径必须拒绝"
    );
    // 相对路径 .. 逃逸（目标不存在——最近祖先解析后落根外）
    assert!(resolve_sandboxed(&ws, Path::new("../escape.md")).is_err());
    // 深逃逸（穿过根内前缀再出根）
    assert!(resolve_sandboxed(&ws, Path::new("src/../../escape.md")).is_err());
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn sandbox_rejects_symlink_escape() {
    let ws = temp_ws("symlink");
    let outside =
        std::env::temp_dir().join(format!("mang-rpc-fs-out-{}-symlink", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, ws.join("link")).unwrap();

    // 链接目标在根外（存在路径——整条 canonicalize 摊平后拒）
    assert!(resolve_sandboxed(&ws, Path::new("link")).is_err());
    // 经链接指向根外的不存在路径（祖先 canonicalize 摊平后拒）
    assert!(resolve_sandboxed(&ws, Path::new("link/newfile.md")).is_err());
    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn sandbox_resolves_nonexistent_tail_via_existing_ancestor() {
    let ws = temp_ws("tail");
    std::fs::create_dir_all(ws.join("src/deep")).unwrap();
    let canon_ws = ws.canonicalize().unwrap();

    // 不存在的文件（父存在）→ canonical 祖先 + 尾段
    assert_eq!(
        resolve_sandboxed(&ws, Path::new("src/main.rs")).unwrap(),
        canon_ws.join("src/main.rs")
    );
    // 多层不存在（父也不存在）→ 逐级上溯到最近存在祖先
    assert_eq!(
        resolve_sandboxed(&ws, Path::new("src/new1/new2/f.rs")).unwrap(),
        canon_ws.join("src/new1/new2/f.rs")
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn sandbox_prefix_is_component_wise_not_string_wise() {
    let ws = temp_ws("prefix");
    // 兄弟目录：字符串前缀相同（ws + "x"）但组件不同——必须拒
    //（字符串级 starts_with 会误放行）。
    let ws_x = ws.with_file_name(format!("{}x", ws.file_name().unwrap().to_string_lossy()));
    let _ = std::fs::remove_dir_all(&ws_x);
    std::fs::create_dir_all(&ws_x).unwrap();
    assert!(
        resolve_sandboxed(&ws, &ws_x).is_err(),
        "组件级前缀：兄弟目录不得因字符串前缀相同而放行"
    );
    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&ws_x);
}

// ---------------- 端点解析 ----------------

#[test]
fn socket_path_explicit_override_wins() {
    let p = default_socket_path(
        Some(PathBuf::from("/tmp/custom.sock")),
        Some(PathBuf::from("/run/user/1000")),
        Some(1000),
    );
    assert_eq!(p, PathBuf::from("/tmp/custom.sock"));
}

#[test]
fn socket_path_xdg_runtime_then_tmp_uid_fallback() {
    let p = default_socket_path(None, Some(PathBuf::from("/run/user/1000")), None);
    assert_eq!(p, PathBuf::from("/run/user/1000/mang-shell.sock"));

    let p = default_socket_path(None, None, Some(1234));
    assert_eq!(p, PathBuf::from("/tmp/mang-shell-1234.sock"));

    let p = default_socket_path(None, None, None);
    assert_eq!(p, PathBuf::from("/tmp/mang-shell.sock"));
}

// ---------------- socket 预备（探活/清残留/防误删） ----------------

/// 测试临时路径（进程内唯一——避开并行测试互踩；G3 全局 config_dir
/// 污染教训：测试不共享可变全局路径）。
fn temp_sock(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("mang-rpc-test-{}-{name}.sock", std::process::id()))
}

#[test]
fn prepare_socket_missing_path_is_ready() {
    let path = temp_sock("missing");
    let _ = std::fs::remove_file(&path);
    assert!(matches!(prepare_socket(&path), Ok(Prepared::Ready)));
}

#[test]
fn prepare_socket_live_listener_is_busy() {
    let path = temp_sock("live");
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    assert!(
        matches!(prepare_socket(&path), Ok(Prepared::Busy)),
        "活实例在听——应报 Busy 而不是清它的文件"
    );
    drop(listener);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn prepare_socket_stale_socket_removed() {
    let path = temp_sock("stale");
    let _ = std::fs::remove_file(&path);
    // 真 socket 残留：绑一个监听后直接丢弃（不 unlink——上次异常
    // 退出的准确模拟；普通文件残留走下面的拒绝测试）。
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    drop(listener);
    assert!(path.exists(), "丢弃监听者不删 socket 文件——残留成立");
    assert!(matches!(prepare_socket(&path), Ok(Prepared::Ready)));
    assert!(!path.exists(), "陈旧残留应被清除");
}

#[test]
fn prepare_socket_refuses_to_delete_non_socket_file() {
    let path =
        std::env::temp_dir().join(format!("mang-rpc-test-{}-regular.txt", std::process::id()));
    std::fs::write(&path, b"important data").unwrap();
    let result = prepare_socket(&path);
    assert!(result.is_err(), "普通文件必须拒绝而不是删除");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "important data",
        "内容不得被破坏"
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------- 行分帧 ----------------

#[test]
fn line_assembler_splits_multiple_commands_in_one_chunk() {
    let mut asm = LineAssembler::new(1024);
    asm.feed(b"{\"id\":1,\"cmd\":\"get_state\"}\n{\"id\":2,\"cmd\":\"get_state\"}\r\n");
    assert_eq!(
        asm.next_line().as_deref(),
        Some(r#"{"id":1,"cmd":"get_state"}"#)
    );
    assert_eq!(
        asm.next_line().as_deref(),
        Some(r#"{"id":2,"cmd":"get_state"}"#),
        "CRLF 行尾归一（\\r 不入命令行）"
    );
    assert_eq!(asm.next_line(), None, "无更多完整行");
    assert!(!asm.overflowed());
}

#[test]
fn line_assembler_partial_then_completed() {
    let mut asm = LineAssembler::new(1024);
    asm.feed(br#"{"id":1,"cmd":"ge"#);
    assert_eq!(asm.next_line(), None, "半行不吐");
    asm.feed(br#"t_state"}"#);
    assert_eq!(asm.next_line(), None, "仍无换行——继续攒");
    asm.feed(b"\n");
    assert_eq!(
        asm.next_line().as_deref(),
        Some(r#"{"id":1,"cmd":"get_state"}"#)
    );
}

#[test]
fn line_assembler_enforces_cap_and_takes_remainder() {
    let mut asm = LineAssembler::new(16);
    asm.feed(&[b'a'; 17]);
    assert!(asm.overflowed(), "无换行待处理字节超上限");
    // 上限以「待处理字节」计：完整行弹出后即清——不误伤合法长行
    let mut asm = LineAssembler::new(16);
    asm.feed(&[b'a'; 10]);
    assert!(!asm.overflowed());
    asm.feed(b"\n");
    assert!(!asm.overflowed(), "完整行在缓冲内不算超限");
    assert!(asm.next_line().is_some());
    assert!(!asm.overflowed());
}

#[test]
fn line_assembler_remainder_without_newline() {
    let mut asm = LineAssembler::new(1024);
    asm.feed(b"{\"id\":3,\"cmd\":\"get_state\"}");
    assert_eq!(asm.next_line(), None);
    assert_eq!(
        asm.take_remainder().as_deref(),
        Some(r#"{"id":3,"cmd":"get_state"}"#),
        "EOF 尾包（无换行的末行）也应被处理"
    );
    assert_eq!(asm.take_remainder(), None);
}
