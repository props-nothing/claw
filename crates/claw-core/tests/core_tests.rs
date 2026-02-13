#[cfg(test)]
mod tests {
    use claw_core::*;
    use uuid::Uuid;

    // ── Message tests ──────────────────────────────────────────

    #[test]
    fn test_message_text_constructor() {
        let sid = Uuid::new_v4();
        let msg = Message::text(sid, Role::User, "hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.text_content(), "hello");
        assert!(msg.tool_calls.is_empty());
        assert_eq!(msg.session_id, sid);
    }

    #[test]
    fn test_message_text_joins_blocks() {
        let sid = Uuid::new_v4();
        let mut msg = Message::text(sid, Role::Assistant, "Hello ");
        msg.content.push(MessageContent::Text { text: "world".to_string() });
        assert_eq!(msg.text_content(), "Hello \nworld");
    }

    #[test]
    fn test_message_text_empty_content() {
        let sid = Uuid::new_v4();
        let mut msg = Message::text(sid, Role::System, "");
        msg.content.clear();
        assert_eq!(msg.text_content(), "");
    }

    #[test]
    fn test_message_serde_roundtrip() {
        let msg = Message::text(Uuid::new_v4(), Role::User, "test message");
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.role, Role::User);
        assert_eq!(restored.text_content(), "test message");
    }

    #[test]
    fn test_role_variants() {
        let roles = [Role::System, Role::User, Role::Assistant, Role::Tool];
        for role in &roles {
            let json = serde_json::to_string(role).unwrap();
            let restored: Role = serde_json::from_str(&json).unwrap();
            assert_eq!(*role, restored);
        }
    }

    // ── Error tests ────────────────────────────────────────────

    #[test]
    fn test_error_display() {
        let err = ClawError::Agent("something broke".into());
        assert!(err.to_string().contains("something broke"));
    }

    #[test]
    fn test_error_rate_limited() {
        let err = ClawError::RateLimited { retry_after_secs: 30 };
        assert!(err.to_string().contains("30"));
    }

    #[test]
    fn test_error_context_overflow() {
        let err = ClawError::ContextOverflow { used: 200000, max: 128000 };
        assert!(err.to_string().contains("200000"));
        assert!(err.to_string().contains("128000"));
    }

    #[test]
    fn test_error_tool_denied() {
        let err = ClawError::ToolDenied {
            tool: "rm".into(),
            reason: "blocked".into(),
        };
        let s = err.to_string();
        assert!(s.contains("rm"));
        assert!(s.contains("blocked"));
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: ClawError = io_err.into();
        assert!(err.to_string().contains("file not found"));
    }

    // ── Tool tests ─────────────────────────────────────────────

    #[test]
    fn test_tool_serde() {
        let tool = Tool {
            name: "shell_exec".into(),
            description: "Run a shell command".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            }),
            capabilities: vec!["execute".into()],
            is_mutating: true,
            risk_level: 8,
            provider: Some("builtin".into()),
        };
        let json = serde_json::to_string(&tool).unwrap();
        let restored: Tool = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "shell_exec");
        assert_eq!(restored.risk_level, 8);
        assert!(restored.is_mutating);
    }

    #[test]
    fn test_tool_call_serde() {
        let tc = ToolCall {
            id: "call_123".into(),
            tool_name: "read_file".into(),
            arguments: serde_json::json!({"path": "/tmp/test"}),
        };
        let json = serde_json::to_string(&tc).unwrap();
        let restored: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.tool_name, "read_file");
    }

    #[test]
    fn test_tool_result_serde() {
        let tr = ToolResult {
            tool_call_id: "call_123".into(),
            content: "file contents".into(),
            is_error: false,
            data: Some(serde_json::json!({"lines": 42})),
        };
        let json = serde_json::to_string(&tr).unwrap();
        let restored: ToolResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.content, "file contents");
        assert!(!restored.is_error);
    }

    // ── Event Bus tests ────────────────────────────────────────

    #[test]
    fn test_event_bus_pub_sub() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        bus.publish(Event::Shutdown);

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, Event::Shutdown));
    }

    #[test]
    fn test_event_bus_multiple_subscribers() {
        let bus = EventBus::default();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        bus.publish(Event::Shutdown);

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn test_event_serde_roundtrip() {
        let event = Event::AgentToolCall {
            session_id: Uuid::new_v4(),
            tool_name: "shell_exec".to_string(),
            tool_call_id: "call_123".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let restored: Event = serde_json::from_str(&json).unwrap();
        if let Event::AgentToolCall { tool_name, tool_call_id, .. } = restored {
            assert_eq!(tool_name, "shell_exec");
            assert_eq!(tool_call_id, "call_123");
        } else {
            panic!("wrong variant");
        }
    }

    // ── DeviceInfo / Os / Arch tests ───────────────────────────

    #[test]
    fn test_os_current() {
        let os = Os::current();
        #[cfg(target_os = "macos")]
        assert_eq!(os, Os::MacOS);
        #[cfg(target_os = "linux")]
        assert_eq!(os, Os::Linux);
    }

    #[test]
    fn test_arch_current() {
        let arch = Arch::current();
        #[cfg(target_arch = "aarch64")]
        assert_eq!(arch, Arch::Aarch64);
        #[cfg(target_arch = "x86_64")]
        assert_eq!(arch, Arch::X86_64);
    }

    // ── MessageContent tests ───────────────────────────────────

    #[test]
    fn test_message_content_variants_serde() {
        let blocks = vec![
            MessageContent::Text { text: "hello".into() },
            MessageContent::Image {
                media_type: "image/png".into(),
                data: "base64data".into(),
            },
            MessageContent::File {
                path: "/tmp/test.txt".into(),
                media_type: Some("text/plain".into()),
            },
        ];
        for block in blocks {
            let json = serde_json::to_string(&block).unwrap();
            let _restored: MessageContent = serde_json::from_str(&json).unwrap();
        }
    }
}
