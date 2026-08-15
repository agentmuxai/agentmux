    use super::*;
    use crate::backend::obj::*;

    fn make_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn test_insert_and_get_client() {
        let store = make_store();
        let mut client = Client {
            oid: "test-client-oid".to_string(),
            version: 0,
            windowids: vec!["w1".to_string()],
            meta: MetaMapType::new(),
            tosagreed: 1700000000000,
            ..Default::default()
        };
        store.insert(&mut client).unwrap();
        assert_eq!(client.get_version(), 1);

        let loaded = store.must_get::<Client>("test-client-oid").unwrap();
        assert_eq!(loaded.oid, "test-client-oid");
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.windowids, vec!["w1"]);
        assert_eq!(loaded.tosagreed, 1700000000000);
    }

    #[test]
    fn test_insert_and_get_window() {
        let store = make_store();
        let mut win = Window {
            oid: "win-1".to_string(),
            workspaceid: "ws-1".to_string(),
            pos: Point { x: 10, y: 20 },
            winsize: WinSize {
                width: 800,
                height: 600,
            },
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut win).unwrap();

        let loaded = store.must_get::<Window>("win-1").unwrap();
        assert_eq!(loaded.workspaceid, "ws-1");
        assert_eq!(loaded.pos.x, 10);
        assert_eq!(loaded.winsize.width, 800);
    }

    #[test]
    fn test_insert_and_get_workspace() {
        let store = make_store();
        let mut ws = Workspace {
            oid: "ws-1".to_string(),
            name: "Test WS".to_string(),
            tabids: vec!["t1".to_string()],
            activetabid: "t1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut ws).unwrap();

        let loaded = store.must_get::<Workspace>("ws-1").unwrap();
        assert_eq!(loaded.name, "Test WS");
        assert_eq!(loaded.tabids, vec!["t1"]);
    }

    #[test]
    fn test_insert_and_get_tab() {
        let store = make_store();
        let mut tab = Tab {
            oid: "tab-1".to_string(),
            name: "Shell".to_string(),
            layoutstate: "ls-1".to_string(),
            blockids: vec!["b1".to_string()],
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut tab).unwrap();

        let loaded = store.must_get::<Tab>("tab-1").unwrap();
        assert_eq!(loaded.name, "Shell");
    }

    #[test]
    fn test_insert_and_get_block() {
        let store = make_store();
        let mut block = Block {
            oid: "blk-1".to_string(),
            parentoref: "tab:tab-1".to_string(),
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".into(), serde_json::json!("term"));
                m
            },
            ..Default::default()
        };
        store.insert(&mut block).unwrap();

        let loaded = store.must_get::<Block>("blk-1").unwrap();
        assert_eq!(loaded.parentoref, "tab:tab-1");
        assert_eq!(loaded.meta.get("view").unwrap(), "term");
    }

    #[test]
    fn test_insert_and_get_layout_state() {
        let store = make_store();
        // Phase E.4.B Phase 2 — uses typed LayoutNode (was a junk JSON blob).
        let mut ls = LayoutState {
            oid: "ls-1".to_string(),
            rootnode: Some(crate::backend::obj::LayoutNode {
                id: "n1".into(),
                flex_direction: crate::backend::obj::FlexDirection::Row,
                size: 1.0,
                children: Vec::new(),
                data: None,
                ..Default::default()
            }),
            magnifiednodeid: "n1".to_string(),
            ..Default::default()
        };
        store.insert(&mut ls).unwrap();

        let loaded = store.must_get::<LayoutState>("ls-1").unwrap();
        assert_eq!(loaded.magnifiednodeid, "n1");
        assert!(loaded.rootnode.is_some());
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let store = make_store();
        let result = store.get::<Client>("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_must_get_nonexistent_returns_error() {
        let store = make_store();
        let result = store.must_get::<Client>("nonexistent");
        assert!(matches!(result, Err(StoreError::NotFound)));
    }

    #[test]
    fn test_update_increments_version() {
        let store = make_store();
        let mut client = Client {
            oid: "c1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut client).unwrap();
        assert_eq!(client.version, 1);

        client.windowids = vec!["w-new".to_string()];
        let v2 = store.update(&mut client).unwrap();
        assert_eq!(v2, 2);
        assert_eq!(client.version, 2);

        let v3 = store.update(&mut client).unwrap();
        assert_eq!(v3, 3);
    }

    #[test]
    fn test_delete() {
        let store = make_store();
        let mut client = Client {
            oid: "del-me".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut client).unwrap();
        assert!(store.get::<Client>("del-me").unwrap().is_some());

        store.delete::<Client>("del-me").unwrap();
        assert!(store.get::<Client>("del-me").unwrap().is_none());
    }

    #[test]
    fn test_get_all() {
        let store = make_store();
        for i in 0..3 {
            let mut tab = Tab {
                oid: format!("tab-{i}"),
                name: format!("Tab {i}"),
                meta: MetaMapType::new(),
                ..Default::default()
            };
            store.insert(&mut tab).unwrap();
        }

        let all = store.get_all::<Tab>().unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_count() {
        let store = make_store();
        assert_eq!(store.count::<Client>().unwrap(), 0);

        let mut c = Client {
            oid: "c1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut c).unwrap();
        assert_eq!(store.count::<Client>().unwrap(), 1);
    }

    #[test]
    fn test_insert_empty_oid_fails() {
        let store = make_store();
        let mut client = Client {
            oid: String::new(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        let result = store.insert(&mut client);
        assert!(matches!(result, Err(StoreError::EmptyOID)));
    }

    #[test]
    fn test_with_tx_commits_on_success() {
        let store = make_store();
        store
            .with_tx(|tx| {
                let mut ws = Workspace {
                    oid: "ws-tx".to_string(),
                    name: "TX Workspace".to_string(),
                    meta: MetaMapType::new(),
                    ..Default::default()
                };
                tx.insert(&mut ws)?;

                let mut tab = Tab {
                    oid: "tab-tx".to_string(),
                    // tabN naming convention per SPEC_TAB_GAPS_AND_NAMING_2026_04_25.
                    name: "tab1".to_string(),
                    layoutstate: "ls-tx".to_string(),
                    meta: MetaMapType::new(),
                    ..Default::default()
                };
                tx.insert(&mut tab)?;

                // Update workspace to reference tab
                ws.tabids.push("tab-tx".to_string());
                tx.update(&mut ws)?;

                Ok(())
            })
            .unwrap();

        // Verify everything committed
        let ws = store.must_get::<Workspace>("ws-tx").unwrap();
        assert_eq!(ws.name, "TX Workspace");
        assert_eq!(ws.tabids, vec!["tab-tx"]);
        assert_eq!(ws.version, 2); // insert=v1, update=v2

        let tab = store.must_get::<Tab>("tab-tx").unwrap();
        assert_eq!(tab.name, "tab1");
    }

    #[test]
    fn test_with_tx_rollbacks_on_error() {
        let store = make_store();
        let result: Result<(), StoreError> = store.with_tx(|tx| {
            let mut ws = Workspace {
                oid: "ws-rollback".to_string(),
                name: "Should Not Exist".to_string(),
                meta: MetaMapType::new(),
                ..Default::default()
            };
            tx.insert(&mut ws)?;

            // Force an error
            Err(StoreError::Other("intentional failure".to_string()))
        });
        assert!(result.is_err());

        // Verify the insert was rolled back
        let ws = store.get::<Workspace>("ws-rollback").unwrap();
        assert!(ws.is_none());
    }

    #[test]
    fn test_agent_def_insert_collision_resolves_at_runtime() {
        // Two agents whose names derive to the same slug must both
        // insert successfully, with the second getting a `-2` suffix.
        // This exercises the runtime collision-resolution path in
        // agent_def_insert (separate from the migration backfill path
        // tested in migrations.rs).
        let store = make_store();

        let mut a1 = AgentDefinition {
            id: "id-a".to_string(),
            slug: String::new(),
            name: "Agent X".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut a1).unwrap();
        // "Agent X" → "agent-x"
        assert_eq!(a1.slug, "agent-x");

        let mut a2 = AgentDefinition {
            id: "id-b".to_string(),
            // Different surface form, derives to the same slug
            name: "agent x".to_string(),
            ..a1.clone()
        };
        a2.slug = String::new();
        store.agent_def_insert(&mut a2).unwrap();
        assert_eq!(a2.slug, "agent-x-2");

        let mut a3 = AgentDefinition {
            id: "id-c".to_string(),
            name: "AGENT-X".to_string(),
            ..a1.clone()
        };
        a3.slug = String::new();
        store.agent_def_insert(&mut a3).unwrap();
        assert_eq!(a3.slug, "agent-x-3");

        // Verify the underlying rows actually got written
        let listed = store.agent_def_list().unwrap();
        let slugs: Vec<&str> = listed.iter().map(|a| a.slug.as_str()).collect();
        assert!(slugs.contains(&"agent-x"));
        assert!(slugs.contains(&"agent-x-2"));
        assert!(slugs.contains(&"agent-x-3"));
    }

    #[test]
    fn test_agent_def_insert_explicit_slug_collision_resolves() {
        // When a caller passes an explicit (non-empty) slug that
        // already exists, agent_def_insert still resolves the collision
        // via suffixing — guards against the seed pre-loading the
        // same slug twice or any other "I know the slug" path.
        let store = make_store();

        let mut a1 = AgentDefinition {
            id: "id-a".to_string(),
            slug: "explicit".to_string(),
            name: "First".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut a1).unwrap();
        assert_eq!(a1.slug, "explicit");

        let mut a2 = AgentDefinition {
            id: "id-b".to_string(),
            ..a1.clone()
        };
        a2.slug = "explicit".to_string();
        store.agent_def_insert(&mut a2).unwrap();
        assert_eq!(a2.slug, "explicit-2");
    }

    // ---- v6 identity / instance CRUD ----

    fn v6_test_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn sample_account(id: &str, provider: &str) -> IdentityAccount {
        IdentityAccount {
            id: id.to_string(),
            name: format!("asaf-{provider}"),
            provider: provider.to_string(),
            kind: "pat".to_string(),
            display_name: "".to_string(),
            secret_ref: SecretRef::Env { env_var: format!("{}_TOKEN", provider.to_uppercase()) },
            context: serde_json::json!({"username": "asaf"}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn sample_agent(id: &str, slug: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            slug: slug.to_string(),
            name: id.to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: "".to_string(),
            working_directory: "".to_string(),
            shell: "".to_string(),
            provider_flags: "".to_string(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: "".to_string(),
            agent_bus_id: "".to_string(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),}
    }

    #[test]
    fn test_identity_upsert_round_trip() {
        let store = v6_test_store();
        let acct = sample_account("id-gh", "github");
        store.identity_upsert(&acct).unwrap();

        let fetched = store.identity_get("id-gh").unwrap().expect("row");
        assert_eq!(fetched.name, "asaf-github");
        assert_eq!(fetched.provider, "github");
        assert!(matches!(fetched.secret_ref, SecretRef::Env { ref env_var } if env_var == "GITHUB_TOKEN"));
        assert_eq!(fetched.context["username"], "asaf");
    }

    #[test]
    fn test_identity_list_filtered_by_provider() {
        let store = v6_test_store();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();
        store.identity_upsert(&sample_account("id-aws", "aws")).unwrap();
        store.identity_upsert(&sample_account("id-gh2", "github")).unwrap();

        let all = store.identity_list(None).unwrap();
        assert_eq!(all.len(), 3);
        let gh = store.identity_list(Some("github")).unwrap();
        assert_eq!(gh.len(), 2);
        assert!(gh.iter().all(|a| a.provider == "github"));
    }

    #[test]
    fn test_identity_delete() {
        let store = v6_test_store();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();
        assert!(store.identity_delete("id-gh").unwrap().deleted);
        assert!(store.identity_get("id-gh").unwrap().is_none());
        // Second delete is a no-op.
        assert!(!store.identity_delete("id-gh").unwrap().deleted);
    }

    #[test]
    fn test_agent_identity_link_and_unlink() {
        let store = v6_test_store();
        let mut agent = sample_agent("ag1", "agent-x");
        store.agent_def_insert(&mut agent).unwrap();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();

        store.agent_identity_link("ag1", "id-gh", "github").unwrap();
        let links = store.agent_identity_list_for_agent("ag1").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].account_id, "id-gh");

        // Re-link with a different account overwrites (one account per provider per agent)
        store.identity_upsert(&sample_account("id-gh2", "github")).unwrap();
        store.agent_identity_link("ag1", "id-gh2", "github").unwrap();
        let links = store.agent_identity_list_for_agent("ag1").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].account_id, "id-gh2");

        assert!(store.agent_identity_unlink("ag1", "github").unwrap());
        assert!(store.agent_identity_list_for_agent("ag1").unwrap().is_empty());
    }

    #[test]
    fn test_agent_identity_cascade_on_agent_delete() {
        let store = v6_test_store();
        let mut agent = sample_agent("ag1", "agent-x");
        store.agent_def_insert(&mut agent).unwrap();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();
        store.agent_identity_link("ag1", "id-gh", "github").unwrap();

        store.agent_def_delete("ag1").unwrap();
        assert!(store.agent_identity_list_for_agent("ag1").unwrap().is_empty());
    }

    // Exercises agent_identity_list_all() across multiple agents and
    // providers, not just the empty/single-agent cases the migration tests
    // (m0013's `is_empty()` assertion) happen to cover. Backs the Armory
    // "Identities" read-only rail (issue #1624 PR-C), the first live RPC
    // consumer of this method beyond the startup-backfill migrations.
    #[test]
    fn test_agent_identity_list_all_spans_every_agent_and_provider() {
        let store = v6_test_store();
        let mut agent1 = sample_agent("ag1", "agent-x");
        let mut agent2 = sample_agent("ag2", "agent-y");
        store.agent_def_insert(&mut agent1).unwrap();
        store.agent_def_insert(&mut agent2).unwrap();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();
        store.identity_upsert(&sample_account("id-claude", "claude")).unwrap();

        store.agent_identity_link("ag1", "id-gh", "github").unwrap();
        store.agent_identity_link("ag1", "id-claude", "claude").unwrap();
        store.agent_identity_link("ag2", "id-claude", "claude").unwrap();

        let mut all = store.agent_identity_list_all().unwrap();
        all.sort_by(|a, b| (a.agent_id.as_str(), a.provider.as_str()).cmp(&(b.agent_id.as_str(), b.provider.as_str())));
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].agent_id, "ag1");
        assert_eq!(all[0].provider, "claude");
        assert_eq!(all[1].agent_id, "ag1");
        assert_eq!(all[1].provider, "github");
        assert_eq!(all[2].agent_id, "ag2");
        assert_eq!(all[2].provider, "claude");
        assert_eq!(all[2].account_id, "id-claude");
    }

    #[test]
    fn test_instance_create_update_filter() {
        let store = v6_test_store();
        let mut agent = sample_agent("def1", "agent-x");
        store.agent_def_insert(&mut agent).unwrap();

        let inst = AgentInstance {
            id: "inst1".to_string(),
            definition_id: "def1".to_string(),
            parent_instance_id: String::new(),
            block_id: "block-abc".to_string(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 1000,
            ended_at: 0,
            created_at: 1000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();

        let fetched = store.instance_get("inst1").unwrap().expect("row");
        assert_eq!(fetched.block_id, "block-abc");
        assert_eq!(fetched.status, "running");

        // Update status → stopped
        let mut updated = fetched.clone();
        updated.status = InstanceStatus::Stopped.as_str().to_string();
        updated.ended_at = 2000;
        assert!(store.instance_update(&updated).unwrap());
        assert_eq!(store.instance_get("inst1").unwrap().unwrap().status, "stopped");

        // Filter queries
        let all = store.instance_list(None, None).unwrap();
        assert_eq!(all.len(), 1);
        let by_def = store.instance_list(Some("def1"), None).unwrap();
        assert_eq!(by_def.len(), 1);
        let running = store.instance_list(None, Some("running")).unwrap();
        assert_eq!(running.len(), 0);
        let stopped = store.instance_list(None, Some("stopped")).unwrap();
        assert_eq!(stopped.len(), 1);
    }

    #[test]
    fn test_instance_update_partial() {
        use crate::backend::storage::InstanceUpdate;
        let store = v6_test_store();
        let mut agent = sample_agent("defp", "agent-p");
        store.agent_def_insert(&mut agent).unwrap();
        let inst = AgentInstance {
            id: "instp".to_string(),
            definition_id: "defp".to_string(),
            parent_instance_id: String::new(),
            block_id: "block-1".to_string(),
            session_id: "sess-1".to_string(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: "ctx-1".to_string(),
            started_at: 1000,
            ended_at: 0,
            created_at: 1000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();

        // Update ONLY status — other columns must be preserved.
        let fresh = store
            .instance_update_partial(
                "instp",
                &InstanceUpdate { status: Some("stopped".into()), ..Default::default() },
            )
            .unwrap()
            .expect("row");
        assert_eq!(fresh.status, "stopped");
        assert_eq!(fresh.block_id, "block-1", "block_id must be untouched");
        assert_eq!(fresh.session_id, "sess-1", "session_id must be untouched");
        assert_eq!(fresh.github_context, "ctx-1", "github_context must be untouched");

        // Update ONLY session_id — status from the prior write persists.
        let fresh = store
            .instance_update_partial(
                "instp",
                &InstanceUpdate { session_id: Some("sess-2".into()), ..Default::default() },
            )
            .unwrap()
            .expect("row");
        assert_eq!(fresh.session_id, "sess-2");
        assert_eq!(fresh.status, "stopped", "status from prior partial write persists");

        // `Some("")` explicitly clears a string column.
        let fresh = store
            .instance_update_partial(
                "instp",
                &InstanceUpdate { github_context: Some(String::new()), ..Default::default() },
            )
            .unwrap()
            .expect("row");
        assert_eq!(fresh.github_context, "", "Some(\"\") clears");

        // All-`None` no-op returns the unchanged row (NOT None — that's
        // reserved for not-found so the handler can tell them apart).
        let noop = store
            .instance_update_partial("instp", &InstanceUpdate::default())
            .unwrap();
        assert!(noop.is_some(), "no-op on an existing id returns the row");
        assert_eq!(noop.unwrap().session_id, "sess-2");

        // Not-found returns None.
        let missing = store
            .instance_update_partial("nope", &InstanceUpdate { status: Some("x".into()), ..Default::default() })
            .unwrap();
        assert!(missing.is_none(), "not-found returns None");
    }

    #[test]
    fn test_instance_get_by_block_id() {
        // SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md §4.1:
        // this lookup is what lets persist_session_id write a live
        // session_id back into db_agent_instances by block_id.
        let store = v6_test_store();
        let mut agent = sample_agent("defq", "agent-q");
        store.agent_def_insert(&mut agent).unwrap();
        let inst = AgentInstance {
            id: "instq".to_string(),
            definition_id: "defq".to_string(),
            parent_instance_id: String::new(),
            block_id: "block-q".to_string(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 1000,
            ended_at: 0,
            created_at: 1000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();

        let found = store.instance_get_by_block_id("block-q").unwrap().expect("row");
        assert_eq!(found.id, "instq");
        assert_eq!(found.session_id, "", "starts empty, matching real launch-time behavior");

        // Writing a live session id through instance_update_partial (as
        // persist_session_id now does) is visible on the next lookup.
        use crate::backend::storage::InstanceUpdate;
        store
            .instance_update_partial("instq", &InstanceUpdate { session_id: Some("sess-live".into()), ..Default::default() })
            .unwrap();
        let found = store.instance_get_by_block_id("block-q").unwrap().expect("row");
        assert_eq!(found.session_id, "sess-live");

        // No row for this block: None, not an error.
        assert!(store.instance_get_by_block_id("no-such-block").unwrap().is_none());
    }

    #[test]
    fn test_agent_def_list_orders_by_last_used() {
        let store = v6_test_store();
        // Three definitions; none launched yet.
        let mut a = sample_agent("def-a", "agent-a");
        let mut b = sample_agent("def-b", "agent-b");
        let mut c = sample_agent("def-c", "agent-c");
        store.agent_def_insert(&mut a).unwrap();
        store.agent_def_insert(&mut b).unwrap();
        store.agent_def_insert(&mut c).unwrap();

        let mk = |id: &str, def: &str, started: i64| AgentInstance {
            id: id.to_string(),
            definition_id: def.to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: started,
            ended_at: 0,
            created_at: started,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        // Launch def-a, then def-b later. def-c is never launched.
        store.instance_create(&mk("i-a", "def-a", 500)).unwrap();
        store.instance_create(&mk("i-b", "def-b", 600)).unwrap();

        let ids = |s: &Store| -> Vec<String> {
            s.agent_def_list().unwrap().into_iter().map(|d| d.id).collect()
        };
        // Most-recently-launched first; never-launched (def-c) last.
        assert_eq!(ids(&store), vec!["def-b", "def-a", "def-c"]);

        // A newer launch of def-a flips it above def-b (MAX(started_at)).
        store.instance_create(&mk("i-a2", "def-a", 700)).unwrap();
        assert_eq!(ids(&store), vec!["def-a", "def-b", "def-c"]);
    }

    #[test]
    fn test_instance_cascade_on_definition_delete() {
        let store = v6_test_store();
        let mut agent = sample_agent("def1", "agent-x");
        store.agent_def_insert(&mut agent).unwrap();
        let inst = AgentInstance {
            id: "inst1".to_string(),
            definition_id: "def1".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();

        store.agent_def_delete("def1").unwrap();
        assert!(store.instance_get("inst1").unwrap().is_none());
    }

    #[test]
    fn test_instance_status_enum_roundtrip() {
        for s in &[
            InstanceStatus::Running,
            InstanceStatus::Paused,
            InstanceStatus::Stopped,
            InstanceStatus::Crashed,
            InstanceStatus::Detached,
        ] {
            assert_eq!(Some(*s), InstanceStatus::parse(s.as_str()));
        }
        assert_eq!(None, InstanceStatus::parse("nonsense"));
    }

    // ── v7 — Memory bundle accessors ─────────────────────────────────────

    #[test]
    fn test_bundle_memory_lifecycle() {
        let store = make_store();

        // Blank singleton always present.
        let initial = store.bundle_memory_list().unwrap();
        assert_eq!(initial.len(), 1);
        assert!(initial[0].is_blank);
        assert_eq!(initial[0].id, "blank");

        // Upsert a user memory.
        let coder = Memory {
            id: "mem-coder".to_string(),
            name: "Claude-coder".to_string(),
            description: "Pair-programming setup".to_string(),
            is_blank: false,
            is_global: false,
            provider: "claude".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            instructions: "You are a careful refactorer.".to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 100,
            updated_at: 100,
        };
        store.bundle_memory_upsert(&coder).unwrap();

        let listed = store.bundle_memory_list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "mem-coder");
        assert_eq!(listed[1].id, "blank");

        let fetched = store.bundle_memory_get("mem-coder").unwrap().unwrap();
        assert_eq!(fetched.provider, "claude");
        assert_eq!(fetched.instructions, "You are a careful refactorer.");

        // Refuse to delete the blank singleton.
        assert!(store.bundle_memory_delete("blank").is_err());

        // Delete the user memory.
        assert!(store.bundle_memory_delete("mem-coder").unwrap());
        assert_eq!(store.bundle_memory_list().unwrap().len(), 1);
    }

    #[test]
    fn test_global_brain_order_and_format() {
        let store = make_store();

        let mk = |id: &str, name: &str, order: i64| Memory {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            is_blank: false,
            is_global: true,
            provider: String::new(),
            model: String::new(),
            instructions: format!("rules for {name}"),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: order,
            created_at: 0,
            updated_at: 0,
        };
        // Insert out of order: B at 0, A at 1.
        store.bundle_memory_upsert(&mk("g-a", "Alpha", 1)).unwrap();
        store.bundle_memory_upsert(&mk("g-b", "Beta", 0)).unwrap();

        // list_global orders by sort_order: Beta (0) then Alpha (1).
        let g = store.bundle_memory_list_global().unwrap();
        assert_eq!(
            g.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["g-b", "g-a"]
        );

        // Reorder to [Alpha, Beta].
        let updated = store
            .bundle_memory_reorder(&["g-a".to_string(), "g-b".to_string()])
            .unwrap();
        assert_eq!(updated, 2);
        let g = store.bundle_memory_list_global().unwrap();
        assert_eq!(
            g.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["g-a", "g-b"]
        );

        // Editing a bundle via upsert must NOT disturb its sort_order.
        let mut edited = g[0].clone();
        edited.instructions = "edited".to_string();
        edited.sort_order = 999; // upsert ON CONFLICT ignores this
        store.bundle_memory_upsert(&edited).unwrap();
        let g = store.bundle_memory_list_global().unwrap();
        assert_eq!(g[0].id, "g-a", "edit must keep position");
        assert_eq!(g[0].sort_order, 0, "sort_order owned by reorder, not upsert");

        // The injection block carries [Workspace] headings in order.
        let block = super::super::format_global_brain_block(&g);
        let expected = "# [Workspace] Alpha\n\nedited\n\n---\n\n# [Workspace] Beta\n\nrules for Beta";
        assert_eq!(block, expected);
    }

    // ---- Registry parallel-write mirror (PR A) ----

    fn make_named_inst(id: &str, name: &str, agents_root: &Path) -> AgentInstance {
        // working_directory must sit under <agents_root>/<slug> so the
        // relative-path resolver picks it up.
        let wd = agents_root.join(format!("{name}-fixture")).to_string_lossy().to_string();
        AgentInstance {
            id: id.to_string(),
            definition_id: "def-mirror".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 1_000,
            ended_at: 0,
            created_at: 900,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: name.to_string(),
            working_directory: wd,
            display_hidden: false,
        }
    }

    fn store_with_registry() -> (tempfile::TempDir, Store, Arc<crate::registry::Registry>) {
        let tmp = tempfile::tempdir().unwrap();
        let agents_root = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let reg_root = agents_root.join("registry");
        let reg = Arc::new(crate::registry::Registry::open(reg_root).unwrap());
        let store = Store::open_in_memory().unwrap();
        store.set_registry(reg.clone());
        // Satisfy the FK from db_agent_instances.definition_id.
        let mut agent = sample_agent("def-mirror", "mirror");
        store.agent_def_insert(&mut agent).unwrap();
        (tmp, store, reg)
    }

    #[test]
    fn instance_create_named_writes_registry_file() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-1", "demo", &agents_root);
        store.instance_create(&inst).unwrap();
        let records = reg.list_active().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].data.instance_id, "inst-1");
        assert_eq!(records[0].data.instance_name, "demo");
        assert_eq!(records[0].data.identity_id, None);
        assert_eq!(records[0].data.memory_id, None);
        assert_eq!(records[0].data.working_dir, "demo-fixture");
        // P0.4: the live mirror stamps the current channel agents base so a
        // different channel can reconstruct the absolute working_directory.
        assert_eq!(
            records[0].data.source_agents_base.as_deref(),
            Some(agents_root.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn registry_agents_base_falls_back_to_registry_parent() {
        // With no explicit base, the accessor returns the registry's parent
        // (= the channel agents dir in the pre-re-root layout). This is why
        // existing mirror tests are unchanged by the decoupling.
        let (_tmp, store, _reg) = store_with_registry();
        let base = store.registry_agents_base().unwrap();
        assert!(base.ends_with("agents"), "fallback is the registry parent");
        assert!(!base.ends_with("registry"));
    }

    #[test]
    fn registry_agents_base_override_decouples_from_registry_parent() {
        // Simulates the P0.3 re-root: the registry lives at a GLOBAL root
        // whose parent (`agents_root()` = shared/agents) is NOT the channel
        // agents dir. The explicit base (the channel agents dir, i.e.
        // AGENTMUX_AGENTS_DIR) must win, so a working_directory under the
        // channel dir still mirrors with the correct relative path instead of
        // being dropped as "not under the registry parent".
        let tmp = tempfile::tempdir().unwrap();
        let global_reg_root = tmp.path().join("shared").join("agents").join("registry");
        let reg = Arc::new(crate::registry::Registry::open(global_reg_root).unwrap());
        let store = Store::open_in_memory().unwrap();
        store.set_registry(reg.clone());
        let channel_agents = tmp.path().join("channels").join("local-x").join("agents");
        std::fs::create_dir_all(&channel_agents).unwrap();
        store.set_registry_agents_base(channel_agents.clone());
        // Sanity: the override is NOT the registry's parent.
        assert_ne!(
            store.registry_agents_base().unwrap(),
            reg.agents_root().unwrap()
        );
        // FK satisfy.
        let mut agent = sample_agent("def-mirror", "mirror");
        store.agent_def_insert(&mut agent).unwrap();
        // Instance working dir under the CHANNEL agents dir.
        let inst = make_named_inst("inst-base", "demo", &channel_agents);
        store.instance_create(&inst).unwrap();
        let records = reg.list_active().unwrap();
        assert_eq!(records.len(), 1, "instance mirrors via the explicit base");
        // Relative path stripped against the channel agents dir, not the
        // registry parent (shared/agents) — which wouldn't contain it.
        assert_eq!(records[0].data.working_dir, "demo-fixture");
    }

    /// Production-shaped store: registry at `<tmp>/shared/agents/registry` (so the
    /// mirror derives the GLOBAL workspace root `<tmp>/agents` via the registry
    /// root's 3rd ancestor), with the per-channel base set to
    /// `<tmp>/channels/<ch>/agents` (= AGENTMUX_AGENTS_DIR).
    fn store_with_global_registry() -> (tempfile::TempDir, Store, Arc<crate::registry::Registry>) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
        let reg_root = tmp.path().join("shared").join("agents").join("registry");
        let reg = Arc::new(crate::registry::Registry::open(reg_root).unwrap());
        let store = Store::open_in_memory().unwrap();
        store.set_registry(reg.clone());
        let channel_agents = tmp.path().join("channels").join("local-x").join("agents");
        std::fs::create_dir_all(&channel_agents).unwrap();
        store.set_registry_agents_base(channel_agents);
        let mut agent = sample_agent("def-mirror", "mirror");
        store.agent_def_insert(&mut agent).unwrap();
        (tmp, store, reg)
    }

    #[test]
    fn mirror_anchors_global_workspace() {
        // The bug this fixes: agent workspaces are GLOBAL (`<home>/agents/<name>`),
        // NOT under the per-channel agents dir. The live mirror must anchor on the
        // global root (derived from the registry root) and mirror the agent — not
        // drop it as "not representable" (the live-write twin of #1393).
        let (tmp, store, reg) = store_with_global_registry();
        let global_agents = tmp.path().join("agents"); // <home>/agents
        let inst = make_named_inst("inst-g", "qooma", &global_agents);
        store.instance_create(&inst).unwrap();
        let records = reg.list_active().unwrap();
        assert_eq!(records.len(), 1, "global workspace must mirror, not be dropped");
        assert_eq!(records[0].data.working_dir, "qooma-fixture");
        assert_eq!(
            records[0].data.source_agents_base.as_deref(),
            Some(global_agents.to_string_lossy().as_ref()),
            "anchored on the GLOBAL workspace root, not the channel base"
        );
    }

    #[test]
    fn mirror_per_channel_legacy_workspace_still_works() {
        // A legacy in-channel workspace (under channels/<ch>/agents, not the global
        // root) still mirrors via the per-channel fallback.
        let (tmp, store, reg) = store_with_global_registry();
        let channel_agents = tmp.path().join("channels").join("local-x").join("agents");
        let inst = make_named_inst("inst-c", "legacy", &channel_agents);
        store.instance_create(&inst).unwrap();
        let records = reg.list_active().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].data.working_dir, "legacy-fixture");
        assert_eq!(
            records[0].data.source_agents_base.as_deref(),
            Some(channel_agents.to_string_lossy().as_ref()),
            "fell back to the per-channel base"
        );
    }

    #[test]
    fn mirror_skips_workspace_under_neither_root() {
        // A workspace under neither the global nor the channel root (a user cwd) is
        // skipped — parity with the migration's skip-unmappable behavior.
        let (tmp, store, reg) = store_with_global_registry();
        let outside = tmp.path().join("projects").join("foo");
        let inst = make_named_inst("inst-o", "stray", &outside);
        store.instance_create(&inst).unwrap();
        assert!(
            reg.list_active().unwrap().is_empty(),
            "non-agent workspace must not be mirrored"
        );
    }

    #[test]
    fn instance_create_unnamed_does_not_mirror() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut inst = make_named_inst("inst-2", "demo2", &agents_root);
        inst.instance_name = String::new(); // unnamed
        store.instance_create(&inst).unwrap();
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn instance_set_hidden_retires_then_unretires() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-3", "demo3", &agents_root);
        store.instance_create(&inst).unwrap();
        store.instance_set_hidden("inst-3", true).unwrap();
        assert!(reg.list_active().unwrap().is_empty());
        store.instance_set_hidden("inst-3", false).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);
    }

    #[test]
    fn instance_update_refreshes_registry_record() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-4", "demo4", &agents_root);
        store.instance_create(&inst).unwrap();
        let mut updated = inst.clone();
        updated.status = "paused".to_string();
        updated.session_id = "sess-xyz".to_string();
        store.instance_update(&updated).unwrap();
        // instance_update doesn't bump last_launched_at_ms (started_at is
        // immutable in the SQL update), so we just verify the record
        // still exists and is reachable.
        let records = reg.list_active().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].data.instance_id, "inst-4");
    }

    #[test]
    fn instance_delete_removes_registry_file() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-5", "demo5", &agents_root);
        store.instance_create(&inst).unwrap();
        store.instance_delete("inst-5").unwrap();
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn instance_create_outside_agents_dir_skips_mirror() {
        let (tmp, store, reg) = store_with_registry();
        let mut inst = make_named_inst("inst-6", "demo6", tmp.path());
        // Override working_dir to live outside any "agents/" segment.
        inst.working_directory = tmp.path().join("projects").join("myrepo").to_string_lossy().to_string();
        store.instance_create(&inst).unwrap();
        // SQL row was written, but mirror is skipped because the working
        // dir can't be expressed as a relative subpath under agents/.
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn instance_create_user_path_with_agents_segment_is_skipped() {
        // Anchored-prefix check: a user-owned workspace at
        // `/home/user/code/agents/myproject` must NOT be mirrored. The
        // pre-fix scan-for-segment logic matched the inner "agents",
        // producing `working_dir = "myproject"` that would resolve to
        // `<shared>/agents/myproject` (wrong) when PR B reads the row.
        let (tmp, store, reg) = store_with_registry();
        // tmp is NOT under the registry's agents root, so this is a
        // user path that happens to include an "agents" component.
        let outside = tmp.path().join("code").join("agents").join("myproject");
        let mut inst = make_named_inst("inst-pathconfuse", "confuse", tmp.path());
        inst.working_directory = outside.to_string_lossy().to_string();
        store.instance_create(&inst).unwrap();
        assert!(reg.list_active().unwrap().is_empty(),
            "user path with inner 'agents' segment must not be mirrored");
    }

    #[test]
    fn instance_create_continuation_row_does_not_mirror() {
        // Registry mirror filter (per the doc comment in
        // registry_upsert_if_named) intentionally lags the SQLite
        // dropdown filter: continuation rows are NOT mirrored to
        // registry, even though `instance_list_named` does return
        // them under Option E. Cross-version dedup is the planned
        // follow-up; until then the registry-sourced read path
        // doesn't have the SQLite ORDER BY/LIMIT affordance and so
        // continues to gate on parent_instance_id == ''.
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let parent = make_named_inst("inst-parent", "demoP", &agents_root);
        store.instance_create(&parent).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);

        let mut child = make_named_inst("inst-child", "demoP", &agents_root);
        child.parent_instance_id = "inst-parent".to_string();
        store.instance_create(&child).unwrap();
        // Still only one record — the continuation row is NOT mirrored.
        let recs = reg.list_active().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].data.instance_id, "inst-parent");
    }

    #[test]
    fn instance_list_named_picker_mode_dedupes_continuation_chain() {
        // Discussion #1095 / SPEC_AGENT_ARCHITECTURE Phase 3b.1.
        // Before this dedup, a user with N continuations of one
        // logical agent saw N rows in "My Agents" (the user-visible
        // "4 Claudes" bug). Picker mode now collapses every chain
        // to its most-recent row.
        //
        // Test shape: head + one continuation, same name. Picker
        // returns ONE row — the continuation (latest started_at).
        // The chain's identity is preserved via `parent_instance_id`
        // on the surviving row.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-head", "Maks", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont = make_named_inst("inst-cont", "Maks", &agents_root);
        cont.parent_instance_id = "inst-head".to_string();
        cont.started_at = 200;
        store.instance_create(&cont).unwrap();

        let picker_rows = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(
            picker_rows.len(),
            1,
            "picker mode must collapse continuation chain to ONE entry"
        );
        assert_eq!(picker_rows[0].id, "inst-cont");
        // The surviving row keeps its real parent_instance_id —
        // callers needing to reconstruct the chain can still do so
        // by walking up from this row.
        assert_eq!(picker_rows[0].parent_instance_id, "inst-head");

        // Definition-scoped picker mode — same dedup behavior.
        let scoped = store
            .instance_list_named(10, Some("def-mirror"), None, true)
            .unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].id, "inst-cont");
    }

    #[test]
    fn instance_list_named_picker_mode_dedupes_long_chain() {
        // Regression for the 2026-05-27 "4 Claudes" report
        // (discussion #1095). The user's `db_agent_instances` had
        // 1 head + 4 continuations of the same agent. Picker mode
        // must return exactly 1 row — the most recent.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-root", "Claude", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        // 4 continuations chaining linearly off the head.
        for (i, parent) in [("c1", "inst-root"), ("c2", "c1"), ("c3", "c2"), ("c4", "c3")] {
            let mut c = make_named_inst(i, "Claude", &agents_root);
            c.parent_instance_id = parent.to_string();
            c.started_at = 100 + (i.chars().last().unwrap().to_digit(10).unwrap() as i64) * 100;
            store.instance_create(&c).unwrap();
        }

        let picker_rows = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(
            picker_rows.len(),
            1,
            "five-row chain (1 head + 4 continuations) must collapse to one entry"
        );
        assert_eq!(picker_rows[0].id, "c4", "newest continuation wins");
    }

    #[test]
    fn instance_get_by_name_reads_from_db_agents() {
        // Phase 3b.2 — the consolidated `db_agents` table is the new
        // authority for named-agent lookups. After `instance_create`'s
        // dual-write, the helper must surface the agent by name with
        // the bindings populated from `db_agents`. Transient runtime
        // fields (block_id, session_id, status, started_at as launch
        // moment, ended_at, parent_instance_id) have no analog in the
        // consolidated row and come back as their type defaults.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-1", "Maks", &agents_root);
        head.identity_id = "id-1".to_string();
        head.memory_id = "mem-1".to_string();
        head.github_context = "ghctx".to_string();
        store.instance_create(&head).unwrap();

        let got = store
            .instance_get_by_name("Maks")
            .unwrap()
            .expect("should find by name");
        // Folded user-clone: the def and the instance share ONE
        // db_agents row keyed by def.id (def-mirror is_seeded=0 in
        // the test fixture). The caller still sees `definition_id`
        // populated — via the COALESCE in the query, an empty
        // parent_template_id resolves to the row's own id.
        assert_eq!(got.id, "def-mirror");
        assert_eq!(got.definition_id, "def-mirror");
        assert_eq!(got.instance_name, "Maks");
        assert_eq!(got.identity_id, "id-1");
        assert_eq!(got.memory_id, "mem-1");
        assert_eq!(got.github_context, "ghctx");
        assert!(!got.display_hidden);
        // Transient fields default to empty / 0 — see doc comment.
        assert_eq!(got.parent_instance_id, "");
        assert_eq!(got.block_id, "");
        assert_eq!(got.session_id, "");
        assert_eq!(got.status, "");
        assert_eq!(got.ended_at, 0);
    }

    #[test]
    fn instance_get_by_name_returns_none_for_missing_name() {
        let (_tmp, store, _reg) = store_with_registry();
        assert!(store.instance_get_by_name("does-not-exist").unwrap().is_none());
    }

    #[test]
    fn instance_get_by_name_excludes_hidden_rows() {
        // user_hidden = 1 (via display_hidden) must filter out — both
        // the launch modal collision detect and ContinueNamed depend
        // on "forgotten" agents being invisible.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut inst = make_named_inst("inst-hidden", "Ghost", &agents_root);
        inst.display_hidden = true;
        store.instance_create(&inst).unwrap();
        assert!(store.instance_get_by_name("Ghost").unwrap().is_none());
    }

    #[test]
    fn instance_get_by_name_empty_input_returns_none() {
        let (_tmp, store, _reg) = store_with_registry();
        assert!(store.instance_get_by_name("").unwrap().is_none());
    }

    #[test]
    fn continuation_mirrors_bindings_into_db_agents_user_clone_path() {
        // Codex P2 on PR #1110: when a named agent is continued with
        // different identity/memory/cwd/github_context, those bindings
        // must reach db_agents — otherwise `instance_get_by_name`
        // (which reads from db_agents) returns the head's stale data.
        // For user-clone defs (is_seeded=0), the projection is keyed
        // by def.id and the existing UPDATE handles both head and
        // continuation.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Original launch with one set of bindings.
        let mut head = make_named_inst("inst-head", "Maks", &agents_root);
        head.identity_id = "id-original".to_string();
        head.memory_id = "mem-original".to_string();
        store.instance_create(&head).unwrap();

        // Continuation with NEW bindings.
        let mut cont = make_named_inst("inst-cont", "Maks", &agents_root);
        cont.parent_instance_id = "inst-head".to_string();
        cont.identity_id = "id-NEW".to_string();
        cont.memory_id = "mem-NEW".to_string();
        cont.github_context = "ghctx-NEW".to_string();
        store.instance_create(&cont).unwrap();

        // The folded db_agents row reflects the continuation's bindings.
        let got = store.instance_get_by_name("Maks").unwrap().expect("found");
        assert_eq!(got.id, "def-mirror");
        assert_eq!(
            got.identity_id, "id-NEW",
            "continuation bindings must overwrite the head's"
        );
        assert_eq!(got.memory_id, "mem-NEW");
        assert_eq!(got.github_context, "ghctx-NEW");
    }

    #[test]
    fn instance_get_by_name_collapses_continuation_chain_to_one_row() {
        // Continuations live in `db_agent_instances` (each launch =
        // one row). The Phase 3a dual-write projects them into ONE
        // canonical row in `db_agents` (keyed on the original head's
        // id, with bindings updated each continuation). So a 4-deep
        // chain with the same name surfaces as exactly one row here —
        // no MRU-row tie-breaking needed at this layer.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let head = make_named_inst("inst-head", "Maks", &agents_root);
        store.instance_create(&head).unwrap();

        let mut cont = make_named_inst("inst-cont", "Maks", &agents_root);
        cont.parent_instance_id = "inst-head".to_string();
        store.instance_create(&cont).unwrap();

        let got = store.instance_get_by_name("Maks").unwrap().expect("found");
        // Only one db_agents row exists for the whole chain (the
        // folded user-clone row keyed by def-mirror); both
        // continuations updated its bindings.
        assert_eq!(got.id, "def-mirror");
        assert_eq!(got.instance_name, "Maks");
    }

    #[test]
    fn instance_get_active_for_block_phase_3b4_resolves_via_block_meta() {
        // Phase 3b.4: instead of filtering db_agent_instances by
        // status, follow block.meta.agentId → db_agents directly.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Create the agent (folds into the def-mirror db_agents row).
        let mut inst = make_named_inst("inst-x", "Maks", &agents_root);
        inst.identity_id = "id-resolved".to_string();
        store.instance_create(&inst).unwrap();

        // Create a Block whose meta points at the agent.
        let mut block = crate::backend::obj::Block {
            oid: "block-1".to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m.insert("agentId".to_string(), serde_json::json!("def-mirror"));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        let got = store
            .instance_get_active_for_block("block-1")
            .unwrap()
            .expect("expected the block's agent to resolve");
        assert_eq!(got.id, "def-mirror");
        assert_eq!(got.identity_id, "id-resolved");
        assert_eq!(got.block_id, "block-1"); // echoed from arg
        // Transient fields default — no status filtering needed.
        assert_eq!(got.status, "");
    }

    #[test]
    fn instance_get_active_for_block_phase_3b4_returns_none_when_block_missing() {
        let (_tmp, store, _reg) = store_with_registry();
        assert!(store
            .instance_get_active_for_block("no-such-block")
            .unwrap()
            .is_none());
    }

    #[test]
    fn instance_get_active_for_block_phase_3b4_returns_none_when_no_agent_id() {
        // Block exists but has no agentId in meta → resolver should
        // see None (no agent bound to this block).
        let (_tmp, store, _reg) = store_with_registry();
        let mut block = crate::backend::obj::Block {
            oid: "block-naked".to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: crate::backend::obj::MetaMapType::new(),
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
        assert!(store
            .instance_get_active_for_block("block-naked")
            .unwrap()
            .is_none());
    }

    #[test]
    fn instance_get_active_for_block_phase_3b4_resolves_template_launch_via_legacy_fallback() {
        // Template launches store `agentId = template.id` in block
        // meta. Templates have `is_template = 1` and are filtered
        // out of the consolidated lookup, so the function falls back
        // to the legacy `db_agent_instances` query to find the inst
        // row that was created for this block.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut tpl = sample_agent("tpl-coder", "tpl-coder");
        tpl.is_seeded = 1;
        store.agent_def_insert(&mut tpl).unwrap();
        let mut inst = make_named_inst("inst-tpl", "FromTemplate", &agents_root);
        inst.definition_id = "tpl-coder".to_string();
        inst.identity_id = "id-template-launch".to_string();
        inst.block_id = "block-tpl".to_string();
        store.instance_create(&inst).unwrap();

        let mut block = crate::backend::obj::Block {
            oid: "block-tpl".to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("agentId".to_string(), serde_json::json!("tpl-coder"));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        let got = store
            .instance_get_active_for_block("block-tpl")
            .unwrap()
            .expect("template-launched block resolves via legacy fallback");
        assert_eq!(got.identity_id, "id-template-launch");
    }

    #[test]
    fn instance_get_active_for_block_phase_3b4_ignores_stale_agent_instance_id() {
        // Codex P1 on PR #1114 round 3: pane reuse leaves
        // `agentInstanceId` stale (only `agentId` is cleared by
        // `backToPicker`). The resolver MUST NOT consult that key —
        // otherwise the prior agent's identity bleeds into the new
        // launch. We don't read `agentInstanceId` at all; the
        // current `agentId` is the source of truth.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Set up two distinct agents on different defs.
        let mut prior_def = sample_agent("def-prior", "prior");
        store.agent_def_insert(&mut prior_def).unwrap();
        let mut prior_inst = make_named_inst("inst-prior", "Prior", &agents_root);
        prior_inst.definition_id = "def-prior".to_string();
        prior_inst.identity_id = "id-PRIOR-DO-NOT-USE".to_string();
        store.instance_create(&prior_inst).unwrap();

        // Current agent (the one the user just launched in the
        // reused pane). def-mirror is the fixture's pre-created def.
        let mut current_inst = make_named_inst("inst-current", "Current", &agents_root);
        current_inst.identity_id = "id-current-correct".to_string();
        store.instance_create(&current_inst).unwrap();

        // Block has stale agentInstanceId pointing at the prior
        // agent, but current agentId. The resolver must use agentId.
        let mut block = crate::backend::obj::Block {
            oid: "block-reused".to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("agentId".to_string(), serde_json::json!("def-mirror"));
                m.insert(
                    "agentInstanceId".to_string(),
                    serde_json::json!("inst-prior"),
                );
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        let got = store
            .instance_get_active_for_block("block-reused")
            .unwrap()
            .expect("must resolve to the current agent, not the stale instance id");
        assert_eq!(got.identity_id, "id-current-correct");
        assert_ne!(got.identity_id, "id-PRIOR-DO-NOT-USE");
    }

    #[test]
    fn instance_get_active_for_block_phase_3b4_resolves_hidden_agents_for_existing_panes() {
        // Codex P2 on PR #1114 round 2: hiding a named agent
        // ("forget") is a picker-visibility concept — the pane that's
        // still bound to that agent must keep resolving credentials.
        // Otherwise the next command would silently fall back to
        // ambient creds the moment the user hides the agent.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut inst = make_named_inst("inst-hide", "ToBeHidden", &agents_root);
        inst.identity_id = "id-still-valid".to_string();
        inst.display_hidden = true; // bound block stays alive
        store.instance_create(&inst).unwrap();

        let mut block = crate::backend::obj::Block {
            oid: "block-hidden-agent".to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("agentId".to_string(), serde_json::json!("def-mirror"));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        let got = store
            .instance_get_active_for_block("block-hidden-agent")
            .unwrap()
            .expect("hidden agent must still resolve for existing pane");
        assert_eq!(got.identity_id, "id-still-valid");
    }

    #[test]
    fn instance_get_active_for_block_phase_3b4_legacy_template_block_falls_back_to_instances() {
        // Codex P2 on PR #1114 round 2: panes launched from a seeded
        // template BEFORE `agentInstanceId` stamping was wired only
        // carry `agentId = <template id>`. The db_agents path filters
        // templates out (is_template = 0), so without a fallback
        // those panes would silently fall back to ambient creds.
        // Recovery: legacy `db_agent_instances` lookup by block_id.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Seeded template + an instance launched off it (the way old
        // launches before agentInstanceId stamping would have done).
        let mut tpl = sample_agent("tpl-legacy", "tpl-legacy");
        tpl.is_seeded = 1;
        store.agent_def_insert(&mut tpl).unwrap();
        let mut inst = make_named_inst("inst-legacy-tpl", "OldTplLaunch", &agents_root);
        inst.definition_id = "tpl-legacy".to_string();
        inst.identity_id = "id-legacy-tpl".to_string();
        inst.block_id = "block-old-tpl".to_string();
        store.instance_create(&inst).unwrap();

        // Block ONLY records the template id (no agentInstanceId).
        let mut block = crate::backend::obj::Block {
            oid: "block-old-tpl".to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("agentId".to_string(), serde_json::json!("tpl-legacy"));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        let got = store
            .instance_get_active_for_block("block-old-tpl")
            .unwrap()
            .expect("legacy template block must resolve via db_agent_instances fallback");
        assert_eq!(got.identity_id, "id-legacy-tpl");
    }

    #[test]
    fn instance_get_active_for_block_phase_3b4_honors_legacy_agent_id_meta_key() {
        // Older blocks may still carry `agent:id` instead of `agentId`.
        // Both keys should resolve.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut inst = make_named_inst("inst-legacy", "Maks", &agents_root);
        store.instance_create(&inst).unwrap();

        let mut block = crate::backend::obj::Block {
            oid: "block-legacy".to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("agent:id".to_string(), serde_json::json!("def-mirror"));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        let got = store
            .instance_get_active_for_block("block-legacy")
            .unwrap()
            .expect("legacy agent:id meta key must resolve");
        assert_eq!(got.id, "def-mirror");
    }

    #[test]
    fn instance_list_phase_3b3a_reads_from_db_agents_no_status_filter() {
        // Phase 3b.3a: with no status filter, `instance_list` reads
        // the consolidated `db_agents` table — continuation chains
        // pre-collapse to one row per logical agent, hidden rows
        // excluded.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Two distinct named agents on the same folded def. Each
        // should surface ONCE (one db_agents row each).
        let a = make_named_inst("inst-a", "Maks", &agents_root);
        store.instance_create(&a).unwrap();
        let b = {
            // Different def so the rows don't collide on def.id.
            let mut def2 = sample_agent("def-mirror-2", "mirror-2");
            store.agent_def_insert(&mut def2).unwrap();
            let mut x = make_named_inst("inst-b", "DSad", &agents_root);
            x.definition_id = "def-mirror-2".to_string();
            x
        };
        store.instance_create(&b).unwrap();
        // Continuation of "Maks" — should NOT add a row.
        let mut cont = make_named_inst("inst-cont", "Maks", &agents_root);
        cont.parent_instance_id = "inst-a".to_string();
        store.instance_create(&cont).unwrap();

        let rows = store.instance_list(None, None).unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.instance_name.as_str()).collect();
        assert_eq!(rows.len(), 2);
        assert!(names.contains(&"Maks"));
        assert!(names.contains(&"DSad"));
        // Transient fields default — see doc comment on instance_list.
        for row in &rows {
            assert_eq!(row.status, "");
            assert_eq!(row.block_id, "");
            assert_eq!(row.session_id, "");
        }
    }

    #[test]
    fn instance_list_phase_3b3a_filters_by_definition_lineage() {
        // The legacy `definition_id` filter matches against
        // `parent_template_id` in the consolidated view. For folded
        // user-clones (where the db_agents id IS the def.id), also
        // match the row's own id — both should resolve the def.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let a = make_named_inst("inst-a", "Maks", &agents_root);
        store.instance_create(&a).unwrap();
        let mut def2 = sample_agent("def-other", "other");
        store.agent_def_insert(&mut def2).unwrap();
        let mut b = make_named_inst("inst-b", "Other", &agents_root);
        b.definition_id = "def-other".to_string();
        store.instance_create(&b).unwrap();

        // Filter by def-mirror — only the Maks row matches.
        let filtered = store.instance_list(Some("def-mirror"), None).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].instance_name, "Maks");

        let filtered = store.instance_list(Some("def-other"), None).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].instance_name, "Other");
    }

    #[test]
    fn instance_list_phase_3b3a_definition_id_filter_matches_agent_id_only() {
        // Codex P2 on PR #1111: the legacy `definition_id` filter
        // conflated "agent identity" with "parent template" because of
        // the old schema split. In the consolidated model the filter
        // matches the agent's own id only — templates aren't agents
        // (they have `is_template = 1`, which is excluded by the
        // outer WHERE) and user-clones of a template are SEPARATE
        // agents with their own id, NOT children of the template.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // A seeded template + a user-clone derived from it + a
        // template-instance launched directly off the template.
        let mut tpl = sample_agent("tpl-coder", "tpl-coder");
        tpl.is_seeded = 1;
        store.agent_def_insert(&mut tpl).unwrap();
        let mut clone = sample_agent("clone-of-tpl", "clone-of-tpl");
        clone.parent_id = "tpl-coder".to_string();
        store.agent_def_insert(&mut clone).unwrap();
        let mut tpl_inst = make_named_inst("inst-direct", "DirectTpl", &agents_root);
        tpl_inst.definition_id = "tpl-coder".to_string();
        store.instance_create(&tpl_inst).unwrap();

        // Filter by the template's id returns empty — templates are
        // not agents, and we no longer follow the parent_template_id
        // backlink (which would over-match).
        let by_tpl = store.instance_list(Some("tpl-coder"), None).unwrap();
        assert!(by_tpl.is_empty(), "template id should not surface any agent");

        // Filter by the user-clone's id returns just the clone row.
        let by_clone = store.instance_list(Some("clone-of-tpl"), None).unwrap();
        assert_eq!(by_clone.len(), 1);
        assert_eq!(by_clone[0].id, "clone-of-tpl");

        // Filter by the template-instance id returns just that inst.
        let by_inst = store.instance_list(Some("inst-direct"), None).unwrap();
        assert_eq!(by_inst.len(), 1);
        assert_eq!(by_inst[0].id, "inst-direct");
        assert_eq!(by_inst[0].instance_name, "DirectTpl");

        // Filter by non-existent id returns empty.
        let none = store.instance_list(Some("does-not-exist"), None).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn instance_list_phase_3b3a_orders_by_updated_at_for_continuations() {
        // Reagent P2 on PR #1111 round 2: continuations bump
        // `db_agents.updated_at` but leave `created_at` at the chain
        // head's original timestamp. `ORDER BY created_at` would rank
        // a fresh agent ahead of an actively-continued older one;
        // `ORDER BY updated_at` preserves "most recent activity
        // first".
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Older agent, first.
        let older_head = make_named_inst("inst-old-head", "Older", &agents_root);
        store.instance_create(&older_head).unwrap();

        // Newer agent (different def so different db_agents row).
        let mut def2 = sample_agent("def-newer", "newer");
        store.agent_def_insert(&mut def2).unwrap();
        let mut newer = make_named_inst("inst-newer", "Newer", &agents_root);
        newer.definition_id = "def-newer".to_string();
        store.instance_create(&newer).unwrap();

        // Continue the OLDER agent — bumps its updated_at past
        // the newer agent's.
        let mut cont = make_named_inst("inst-old-cont", "Older", &agents_root);
        cont.parent_instance_id = "inst-old-head".to_string();
        store.instance_create(&cont).unwrap();

        let rows = store.instance_list(None, None).unwrap();
        assert_eq!(rows.len(), 2);
        // Older's continuation made it most recent → it ranks first.
        assert_eq!(rows[0].instance_name, "Older");
        assert_eq!(rows[1].instance_name, "Newer");
    }

    #[test]
    fn instance_list_phase_3b3a_definition_id_projects_row_id_for_user_clones_with_template_lineage() {
        // Reagent P2 on PR #1111 round 2: user-clones derived from a
        // template have `parent_template_id` SET (lineage), but their
        // legacy `definition_id` was the clone's OWN def id, not the
        // template's. The projection must yield the row's id, not
        // walk parent_template_id.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // A user-clone def whose parent_id points at a (non-existent)
        // template — what matters is that parent_template_id ends up
        // SET on the db_agents row.
        let mut clone = sample_agent("clone-from-tpl", "clone-from-tpl");
        clone.parent_id = "tpl-some-template".to_string();
        store.agent_def_insert(&mut clone).unwrap();
        let mut launch = make_named_inst("inst-1", "ClonedAgent", &agents_root);
        launch.definition_id = "clone-from-tpl".to_string();
        store.instance_create(&launch).unwrap();

        let rows = store.instance_list(None, None).unwrap();
        let row = rows.iter().find(|r| r.instance_name == "ClonedAgent").unwrap();
        // definition_id is the CLONE's id, not the template id —
        // even though parent_template_id on the underlying row is set.
        assert_eq!(row.definition_id, "clone-from-tpl");
        assert_eq!(row.id, "clone-from-tpl");
    }

    #[test]
    fn instance_list_phase_3b3a_excludes_hidden() {
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut inst = make_named_inst("inst-h", "Ghost", &agents_root);
        inst.display_hidden = true;
        store.instance_create(&inst).unwrap();
        let rows = store.instance_list(None, None).unwrap();
        assert!(rows.is_empty(), "hidden rows must not surface");
    }

    #[test]
    fn instance_list_phase_3b3b_status_filter_falls_back_to_legacy() {
        // Status filter implies the caller needs transient runtime
        // state. Until the updateagentinstance fetch+merge pattern is
        // refactored, that read must come from db_agent_instances.
        // Verify the legacy path is exercised end-to-end (rows include
        // status field populated from the raw instance row).
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut a = make_named_inst("inst-a", "Maks", &agents_root);
        a.status = "running".to_string();
        store.instance_create(&a).unwrap();

        let running = store.instance_list(None, Some("running")).unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].status, "running");
        assert_eq!(running[0].id, "inst-a"); // raw inst id, NOT the folded def.id

        let stopped = store.instance_list(None, Some("stopped")).unwrap();
        assert!(stopped.is_empty());
    }

    #[test]
    fn instance_list_named_picker_mode_keeps_distinct_agents_separate() {
        // Two unrelated chains (different agents, different names)
        // remain as two rows. The dedup is per-chain, not per-name.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head_a = make_named_inst("a-head", "AgentA", &agents_root);
        head_a.started_at = 100;
        store.instance_create(&head_a).unwrap();
        let mut cont_a = make_named_inst("a-cont", "AgentA", &agents_root);
        cont_a.parent_instance_id = "a-head".to_string();
        cont_a.started_at = 150;
        store.instance_create(&cont_a).unwrap();

        let mut head_b = make_named_inst("b-head", "AgentB", &agents_root);
        head_b.started_at = 200;
        store.instance_create(&head_b).unwrap();

        let rows = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(rows.len(), 2);
        // MRU order: b-head (200) before a-cont (150).
        assert_eq!(rows[0].id, "b-head");
        assert_eq!(rows[1].id, "a-cont");
    }

    #[test]
    fn instance_list_named_picker_mode_orphan_continuation_surfaces() {
        // Regression for codex P2 on PR #1096 bbe897cc: when a chain
        // head is hard-deleted via `deleteagentinstance` (no FK
        // cascade on `parent_instance_id`), descendant continuation
        // rows are orphaned — `parent_instance_id` points at an id
        // that no longer exists.
        //
        // The recursive CTE anchor must seed from BOTH (a) real
        // heads (`parent_instance_id = ''`) and (b) orphans (parent
        // doesn't exist in the table). Without the orphan anchor,
        // the recursive walk can't reach them and they disappear
        // from My Agents — even though they're recoverable sessions
        // the previous (buggy) query surfaced.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Seed a chain: head + 2 continuations.
        let mut head = make_named_inst("inst-deleted-head", "Claude", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont1 = make_named_inst("inst-orphan-cont1", "Claude", &agents_root);
        cont1.parent_instance_id = "inst-deleted-head".to_string();
        cont1.started_at = 200;
        store.instance_create(&cont1).unwrap();

        let mut cont2 = make_named_inst("inst-orphan-cont2", "Claude", &agents_root);
        cont2.parent_instance_id = "inst-orphan-cont1".to_string();
        cont2.started_at = 300;
        store.instance_create(&cont2).unwrap();

        // Hard-delete the head — no cascade, so cont1 + cont2 are
        // now orphaned (cont1.parent_instance_id points at the
        // deleted head; cont2 still has a valid parent).
        store.instance_delete("inst-deleted-head").unwrap();

        let rows = store.instance_list_named(10, None, None, true).unwrap();
        // The orphan chain (cont1 → cont2) must surface as ONE row:
        // cont1 becomes a root (its parent is gone); cont2 chains
        // off cont1. Newest in chain (cont2) wins.
        assert_eq!(
            rows.len(),
            1,
            "orphan chain must remain reachable after head deletion"
        );
        assert_eq!(rows[0].id, "inst-orphan-cont2");
    }

    #[test]
    fn instance_list_named_picker_mode_forget_suppresses_whole_chain() {
        // Regression for codex P2 on PR #1096: when the user clicks
        // "Forget" on a continuation row that's currently the picker's
        // surfaced entry, `hidenamedagent` flips `display_hidden=1`
        // only on that one row. If the dedup query filtered hidden
        // BEFORE ranking, the next-newest visible row in the same
        // chain would inherit `rn = 1` and the "forgotten" agent
        // would immediately reappear — making forget a no-op.
        //
        // Correct behavior: filter hidden AFTER ranking. When the
        // surfaced row is hidden, the entire chain disappears from
        // the picker until the user explicitly unhides one.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-head", "Claude", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont1 = make_named_inst("inst-cont1", "Claude", &agents_root);
        cont1.parent_instance_id = "inst-head".to_string();
        cont1.started_at = 200;
        store.instance_create(&cont1).unwrap();

        let mut cont2 = make_named_inst("inst-cont2", "Claude", &agents_root);
        cont2.parent_instance_id = "inst-cont1".to_string();
        cont2.started_at = 300;
        store.instance_create(&cont2).unwrap();

        // Before forget: chain surfaces as cont2 (newest).
        let before = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].id, "inst-cont2");

        // User clicks "Forget" on the surfaced row.
        store.instance_set_hidden("inst-cont2", true).unwrap();

        // After forget: the whole chain must stay forgotten — older
        // visible rows in the chain (head, cont1) must NOT bubble up.
        let after = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(
            after.len(),
            0,
            "hiding the surfaced row must suppress the entire chain — \
             older visible siblings must NOT be promoted to rn=1"
        );
    }

    #[test]
    fn instance_list_named_picker_mode_skips_hidden_chains() {
        // A hidden continuation should not win the ranking; its
        // sibling (if any) does. If the entire chain is hidden,
        // it disappears entirely.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Chain 1: head + cont, both visible.
        let mut head = make_named_inst("v-head", "Visible", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();
        let mut cont = make_named_inst("v-cont", "Visible", &agents_root);
        cont.parent_instance_id = "v-head".to_string();
        cont.started_at = 200;
        store.instance_create(&cont).unwrap();

        // Chain 2: head + cont, both hidden.
        let mut hidden_head = make_named_inst("h-head", "Hidden", &agents_root);
        hidden_head.started_at = 50;
        hidden_head.display_hidden = true;
        store.instance_create(&hidden_head).unwrap();
        let mut hidden_cont = make_named_inst("h-cont", "Hidden", &agents_root);
        hidden_cont.parent_instance_id = "h-head".to_string();
        hidden_cont.started_at = 60;
        hidden_cont.display_hidden = true;
        store.instance_create(&hidden_cont).unwrap();

        let rows = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "v-cont");
    }

    #[test]
    fn instance_list_named_picker_mode_identity_filter_in_ranking() {
        // Codex P2 #3 on PR #1096 0c4c8c46: identity_id filter must
        // participate in the dedup ranking. If we returned the newest
        // row in a chain and then filtered identity, a chain whose
        // newest row used a different identity would disappear from
        // the picker — even if an older row in the chain matched the
        // requested identity. Push the filter INTO the CTE so the
        // newest IDENTITY-MATCHING row per chain wins.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Chain: head with identity-a, cont with identity-b, then
        // another cont with identity-a (the user switched back).
        let mut head = make_named_inst("inst-head", "Claude", &agents_root);
        head.identity_id = "identity-a".to_string();
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont_b = make_named_inst("inst-cont-b", "Claude", &agents_root);
        cont_b.parent_instance_id = "inst-head".to_string();
        cont_b.identity_id = "identity-b".to_string();
        cont_b.started_at = 200;
        store.instance_create(&cont_b).unwrap();

        let mut cont_a2 = make_named_inst("inst-cont-a2", "Claude", &agents_root);
        cont_a2.parent_instance_id = "inst-cont-b".to_string();
        cont_a2.identity_id = "identity-a".to_string();
        cont_a2.started_at = 300;
        store.instance_create(&cont_a2).unwrap();

        // Filter by identity-a → newest identity-a row wins (cont-a2).
        let rows_a = store
            .instance_list_named(10, None, Some("identity-a"), true)
            .unwrap();
        assert_eq!(rows_a.len(), 1);
        assert_eq!(rows_a[0].id, "inst-cont-a2");

        // Filter by identity-b → only cont-b matches.
        let rows_b = store
            .instance_list_named(10, None, Some("identity-b"), true)
            .unwrap();
        assert_eq!(rows_b.len(), 1);
        assert_eq!(rows_b[0].id, "inst-cont-b");

        // No filter → newest in chain wins (cont-a2, started_at=300).
        let rows_all = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(rows_all.len(), 1);
        assert_eq!(rows_all[0].id, "inst-cont-a2");
    }

    #[test]
    fn instance_list_named_picker_mode_identity_filter_recovers_older_match() {
        // Concrete repro for the bug codex described: chain where the
        // newest row uses identity-b but only older rows match the
        // requested identity-a. Without the in-ranking filter, the
        // chain would disappear when filtering by identity-a (newest
        // row is identity-b, gets ranked first, then post-filter
        // drops it). With the in-ranking filter, identity-a's older
        // row survives because it's the only candidate.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-head", "Claude", &agents_root);
        head.identity_id = "identity-a".to_string();
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont_newer_b = make_named_inst("inst-cont-newer-b", "Claude", &agents_root);
        cont_newer_b.parent_instance_id = "inst-head".to_string();
        cont_newer_b.identity_id = "identity-b".to_string();
        cont_newer_b.started_at = 200;
        store.instance_create(&cont_newer_b).unwrap();

        let rows = store
            .instance_list_named(10, None, Some("identity-a"), true)
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "older identity-a row must survive even though the newest row uses identity-b"
        );
        assert_eq!(rows[0].id, "inst-head");
    }

    #[test]
    fn instance_list_named_dropdown_mode_excludes_continuations() {
        // Launch-modal "Continue agent" dropdown / `listnamedagents`
        // registry-enrichment path: `include_continuations = false`.
        // Symmetric with `registry_upsert_if_named`'s mirror filter —
        // a chain shows up as ONE entry (the head), not N entries
        // for every resume. Codex P1 on PR #1016 first cut: when the
        // enrichment path lost this filter, continuation rows could
        // displace registry-head rows under the `limit` truncation
        // and miss the merge-by-id enrichment.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-head", "Maks", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();
        let mut cont = make_named_inst("inst-cont", "Maks", &agents_root);
        cont.parent_instance_id = "inst-head".to_string();
        cont.started_at = 200;
        store.instance_create(&cont).unwrap();

        let dropdown_rows = store.instance_list_named(10, None, None, false).unwrap();
        assert_eq!(
            dropdown_rows.len(),
            1,
            "legacy dropdown mode must drop continuation rows"
        );
        assert_eq!(dropdown_rows[0].id, "inst-head");

        // Definition-scoped dropdown mode — head only.
        let scoped_dropdown = store
            .instance_list_named(10, Some("def-mirror"), None, false)
            .unwrap();
        assert_eq!(scoped_dropdown.len(), 1);
        assert_eq!(scoped_dropdown[0].id, "inst-head");
    }

    #[test]
    fn instance_update_does_not_resurrect_hidden_row() {
        // Sequence: create (active) → set_hidden(true) → update.
        // The update must NOT move the file from retired/ back to active.
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-resurrect", "demoR", &agents_root);
        store.instance_create(&inst).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);

        store.instance_set_hidden("inst-resurrect", true).unwrap();
        assert!(reg.list_active().unwrap().is_empty(),
            "after set_hidden(true), record must be in retired/");
        assert!(reg.root().join("retired").join("inst-resurrect.json").exists());

        // SQLite still has the row (display_hidden=1). instance_update
        // would refresh it — the mirror must NOT re-add to active.
        let mut updated = inst.clone();
        updated.status = "stopped".to_string();
        updated.ended_at = 9999;
        store.instance_update(&updated).unwrap();

        assert!(reg.list_active().unwrap().is_empty(),
            "instance_update on a hidden row must NOT resurrect it");
        assert!(reg.root().join("retired").join("inst-resurrect.json").exists());
    }

    #[test]
    fn instance_create_with_display_hidden_writes_retired() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut inst = make_named_inst("inst-bornhidden", "demoH", &agents_root);
        inst.display_hidden = true;
        store.instance_create(&inst).unwrap();
        assert!(reg.list_active().unwrap().is_empty());
        assert!(reg.root().join("retired").join("inst-bornhidden.json").exists());
    }

    #[test]
    fn instance_update_toggling_hidden_off_unretires() {
        // Sequence: create → set_hidden(true) → set_hidden(false) →
        // update. After the toggle off, the file should be in active.
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-toggle", "demoT", &agents_root);
        store.instance_create(&inst).unwrap();
        store.instance_set_hidden("inst-toggle", true).unwrap();
        store.instance_set_hidden("inst-toggle", false).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);

        // A subsequent update should preserve active state.
        let mut updated = inst.clone();
        updated.status = "paused".to_string();
        store.instance_update(&updated).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);
        assert!(!reg.root().join("retired").join("inst-toggle.json").exists(),
            "no orphan retired file alongside active");
    }

    #[test]
    fn instance_set_hidden_acts_on_registry_only_row() {
        // Cross-version case: a registry record exists (e.g. migrated
        // from another version's SQLite) but the current version's
        // SQLite has no matching row. `instance_set_hidden` must still
        // flip the registry file and report success.
        let (tmp, store, reg) = store_with_registry();
        // Seed a registry record directly — no SQLite row.
        let agents_root = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let wd = agents_root.join("cross-ver");
        std::fs::create_dir_all(&wd).unwrap();
        reg.upsert(&crate::registry::NamedAgentRecord {
            schema_version: crate::registry::MAX_SUPPORTED_SCHEMA,
            data: crate::registry::NamedAgentRecordV1 {
                instance_id: "inst-crossver".to_string(),
                instance_name: "crossver".to_string(),
                definition_id: "claude-code".to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: "cross-ver".to_string(),
                source_agents_base: None,
                created_at_ms: 100,
                last_launched_at_ms: 100,
                created_by_version: "0.33.821".to_string(),
                last_launched_by_version: "0.33.821".to_string(),
            },
        })
        .unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);
        assert!(store.instance_get("inst-crossver").unwrap().is_none(),
            "precondition: no SQLite row for cross-version agent");

        let result = store.instance_set_hidden("inst-crossver", true).unwrap();
        assert!(result, "must report success even when only registry was affected");
        assert!(reg.list_active().unwrap().is_empty(),
            "registry record must be retired");
        assert!(reg.root().join("retired").join("inst-crossver.json").exists());
    }

    #[test]
    fn agent_def_delete_cascade_removes_registry_files() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst_a = make_named_inst("inst-cascade-a", "demoA", &agents_root);
        let inst_b = make_named_inst("inst-cascade-b", "demoB", &agents_root);
        store.instance_create(&inst_a).unwrap();
        store.instance_create(&inst_b).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 2);

        // Delete the agent definition — SQLite FK cascades both instance
        // rows; the mirror must also drop both registry files.
        store.agent_def_delete("def-mirror").unwrap();
        assert!(reg.list_active().unwrap().is_empty(),
            "agent_def_delete cascade must remove all child instance registry files");
    }

    // ----------------------------------------------------------------
    // Phase 3a — db_agents dual-write coverage
    // ----------------------------------------------------------------

    fn count_agents(store: &Store, where_clause: &str) -> i64 {
        let conn = store.conn.lock().unwrap();
        let sql = format!("SELECT COUNT(*) FROM db_agents WHERE {where_clause}");
        conn.query_row(&sql, [], |row| row.get(0)).unwrap()
    }

    fn read_agent_field(store: &Store, id: &str, field: &str) -> Option<String> {
        let conn = store.conn.lock().unwrap();
        let sql = format!("SELECT {field} FROM db_agents WHERE id = ?1");
        let mut stmt = conn.prepare(&sql).unwrap();
        let r = stmt.query_row(params![id], |row| row.get::<_, String>(0));
        match r {
            Ok(s) => Some(s),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => panic!("query failed: {e}"),
        }
    }

    fn read_agent_int(store: &Store, id: &str, field: &str) -> Option<i64> {
        let conn = store.conn.lock().unwrap();
        let sql = format!("SELECT {field} FROM db_agents WHERE id = ?1");
        let mut stmt = conn.prepare(&sql).unwrap();
        let r = stmt.query_row(params![id], |row| row.get::<_, i64>(0));
        match r {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => panic!("query failed: {e}"),
        }
    }

    #[test]
    fn dual_write_agent_def_insert_seeded_creates_template_row() {
        let store = make_store();
        let mut def = AgentDefinition {
            id: "tpl-dw-seeded".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: "desc".to_string(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut def).unwrap();
        // db_agents row exists, projected as template.
        assert_eq!(read_agent_int(&store, "tpl-dw-seeded", "is_template"), Some(1));
        assert_eq!(read_agent_field(&store, "tpl-dw-seeded", "parent_template_id"), Some(String::new()));
        assert_eq!(read_agent_field(&store, "tpl-dw-seeded", "name"), Some("Coder".to_string()));
        assert_eq!(read_agent_field(&store, "tpl-dw-seeded", "provider"), Some("claude".to_string()));
    }

    #[test]
    fn dual_write_agent_def_insert_user_clone_carries_parent() {
        let store = make_store();
        // Seed the template first so the FK exists in the old table.
        let mut tpl = AgentDefinition {
            id: "tpl-parent".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut tpl).unwrap();
        // User-cloned def has is_seeded=0 + parent_id pointing at template.
        let mut user_def = tpl.clone();
        user_def.id = "def-user".to_string();
        user_def.slug = String::new();
        user_def.is_seeded = 0;
        user_def.parent_id = "tpl-parent".to_string();
        store.agent_def_insert(&mut user_def).unwrap();
        assert_eq!(read_agent_int(&store, "def-user", "is_template"), Some(0));
        assert_eq!(
            read_agent_field(&store, "def-user", "parent_template_id"),
            Some("tpl-parent".to_string())
        );
    }

    #[test]
    fn dual_write_agent_def_update_refreshes_name_in_db_agents() {
        let store = make_store();
        let mut def = AgentDefinition {
            id: "tpl-update".to_string(),
            slug: String::new(),
            name: "Old Name".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut def).unwrap();
        def.name = "New Name".to_string();
        assert!(store.agent_def_update(&mut def).unwrap());
        assert_eq!(
            read_agent_field(&store, "tpl-update", "name"),
            Some("New Name".to_string())
        );
    }

    // Reagent P0 on PR #2282: `agent_def_update`'s legacy `db_agent_definitions`
    // UPDATE never touches `branch_label` — but the dual-write upsert into
    // `db_agents` (the table `agent_def_list` actually reads) does, via its
    // `branch_label = excluded.branch_label` ON CONFLICT clause. Verifies the
    // end-to-end round-trip `renameagentdefinitiontitle` depends on: a
    // `branch_label` change made through `agent_def_update` is both persisted
    // in `db_agents` and visible through `agent_def_list`.
    #[test]
    fn dual_write_agent_def_update_persists_branch_label_change() {
        let store = make_store();
        let mut def = AgentDefinition {
            id: "fork-rename-test".to_string(),
            slug: String::new(),
            name: "Fork Name".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: "some-parent".to_string(),
            branch_label: "Old Branch".to_string(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut def).unwrap();
        def.branch_label = "New Branch".to_string();
        assert!(store.agent_def_update(&mut def).unwrap());
        assert_eq!(
            read_agent_field(&store, "fork-rename-test", "branch_label"),
            Some("New Branch".to_string()),
            "branch_label should persist into db_agents after agent_def_update"
        );
        let listed = store.agent_def_list().unwrap();
        let row = listed.iter().find(|a| a.id == "fork-rename-test").unwrap();
        assert_eq!(row.branch_label, "New Branch", "agent_def_list should reflect the new branch_label");
    }

    #[test]
    fn dual_write_agent_def_delete_removes_db_agents_row() {
        let store = make_store();
        let mut def = AgentDefinition {
            id: "tpl-del".to_string(),
            slug: String::new(),
            name: "Goner".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut def).unwrap();
        assert_eq!(count_agents(&store, "id = 'tpl-del'"), 1);
        store.agent_def_delete("tpl-del").unwrap();
        assert_eq!(count_agents(&store, "id = 'tpl-del'"), 0);
    }

    #[test]
    fn dual_write_instance_create_inserts_user_clone_row() {
        let store = make_store();
        // Seed template.
        let mut tpl = AgentDefinition {
            id: "tpl-for-inst".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: "desc".to_string(),
            working_directory: "/wd/tpl-cfg".to_string(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut tpl).unwrap();

        let inst = AgentInstance {
            id: "inst-dw".to_string(),
            definition_id: "tpl-for-inst".to_string(),
            parent_instance_id: String::new(),
            block_id: "blk-head".to_string(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: "id-1".to_string(),
            memory_id: "mem-1".to_string(),
            instance_name: "Maks".to_string(),
            working_directory: "/wd/maks".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        // Projected row: id == inst.id, is_template = 0, parent = tpl id,
        // bindings copied.
        assert_eq!(read_agent_int(&store, "inst-dw", "is_template"), Some(0));
        assert_eq!(
            read_agent_field(&store, "inst-dw", "parent_template_id"),
            Some("tpl-for-inst".to_string())
        );
        assert_eq!(read_agent_field(&store, "inst-dw", "name"), Some("Maks".to_string()));
        assert_eq!(read_agent_field(&store, "inst-dw", "identity_id"), Some("id-1".to_string()));
        assert_eq!(read_agent_field(&store, "inst-dw", "memory_id"), Some("mem-1".to_string()));
        // working_directory mirrors the DEFINITION's configured cwd, NOT the
        // instance's resolved workdir ("/wd/maks"). db_agents holds durable
        // agent config; the per-launch resolved cwd lives on the block.
        assert_eq!(read_agent_field(&store, "inst-dw", "working_directory"), Some("/wd/tpl-cfg".to_string()));
        // last_block_id mirrors the instance's per-launch block (the one
        // transient field db_agents retains, so My Agents can locate the
        // filestore snapshot). Non-empty value → non-vacuous assertion.
        assert_eq!(read_agent_field(&store, "inst-dw", "last_block_id"), Some("blk-head".to_string()));
        // Continuation rows skipped.
        let cont = AgentInstance {
            id: "inst-cont".to_string(),
            parent_instance_id: "inst-dw".to_string(),
            ..inst.clone()
        };
        store.instance_create(&cont).unwrap();
        assert_eq!(count_agents(&store, "id = 'inst-cont'"), 0);
    }

    /// Reagent P1 + P2 on #1013 round 2 — pins the user-cloned-def
    /// branch of `agents_dual_write_instance_create` so it matches
    /// the backfill rule (`agents_consolidate.rs::backfill_instances`
    /// folds the instance's bindings into the EXISTING `db_agents`
    /// row keyed by `def.id`, NOT a fresh row keyed by `inst.id`).
    /// Round-1 test only covered the seeded-template branch.
    #[test]
    fn dual_write_instance_create_folds_into_user_clone_def() {
        let store = make_store();
        // Seed a template.
        let mut tpl = AgentDefinition {
            id: "tpl-folded".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: "desc".to_string(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut tpl).unwrap();

        // User-clone of the template (is_seeded = 0, parent_id = tpl id).
        let mut clone = AgentDefinition {
            id: "user-clone-1".to_string(),
            slug: String::new(),
            name: "Maks".to_string(),
            is_seeded: 0,
            parent_id: "tpl-folded".to_string(),
            working_directory: "/wd/clone-cfg".to_string(),
            created_at: 1500,
            updated_at: 1500,
            ..tpl.clone()
        };
        store.agent_def_insert(&mut clone).unwrap();
        // The user-clone projection in db_agents starts with empty bindings.
        assert_eq!(read_agent_field(&store, "user-clone-1", "identity_id"), Some(String::new()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "memory_id"), Some(String::new()));

        // Create an instance ON the user-clone def. Per backfill rule,
        // this must FOLD the instance's bindings into the existing
        // user-clone-1 row — NOT create a separate inst-fold-1 row
        // with parent_template_id pointing at a non-template row.
        let inst = AgentInstance {
            id: "inst-fold-1".to_string(),
            definition_id: "user-clone-1".to_string(),
            parent_instance_id: String::new(),
            block_id: "blk-fold".to_string(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: "gh-ctx-A".to_string(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: "id-folded".to_string(),
            memory_id: "mem-folded".to_string(),
            instance_name: "Maks v2".to_string(),
            working_directory: "/wd/folded".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();

        // No new row keyed by inst.id — backfill never creates one for
        // user-clone-def instances, and dual-write must match.
        assert_eq!(count_agents(&store, "id = 'inst-fold-1'"), 0);

        // Bindings folded onto the user-clone-1 row.
        assert_eq!(read_agent_field(&store, "user-clone-1", "identity_id"), Some("id-folded".to_string()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "memory_id"), Some("mem-folded".to_string()));
        // identity_id / memory_id DO fold (per-instance bindings), but
        // working_directory does NOT — it stays the clone def's configured
        // cwd, not the instance's resolved workdir ("/wd/folded").
        assert_eq!(read_agent_field(&store, "user-clone-1", "working_directory"), Some("/wd/clone-cfg".to_string()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "github_context"), Some("gh-ctx-A".to_string()));
        // last_block_id folds onto the user-clone row too (transient
        // per-launch field; non-empty → non-vacuous).
        assert_eq!(read_agent_field(&store, "user-clone-1", "last_block_id"), Some("blk-fold".to_string()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "instance_name"), Some("Maks v2".to_string()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "name"), Some("Maks v2".to_string()));
        // is_template stays 0, parent_template_id untouched (still empty
        // since user-clone insert leaves it blank).
        assert_eq!(read_agent_int(&store, "user-clone-1", "is_template"), Some(0));
    }

    #[test]
    fn dual_write_instance_set_hidden_flips_user_hidden_bit() {
        let store = make_store();
        let mut tpl = AgentDefinition {
            id: "tpl-hide".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut tpl).unwrap();
        let inst = AgentInstance {
            id: "inst-hide".to_string(),
            definition_id: "tpl-hide".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: "H".to_string(),
            working_directory: "/wd/h".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        assert_eq!(read_agent_int(&store, "inst-hide", "user_hidden"), Some(0));
        store.instance_set_hidden("inst-hide", true).unwrap();
        assert_eq!(read_agent_int(&store, "inst-hide", "user_hidden"), Some(1));
        store.instance_set_hidden("inst-hide", false).unwrap();
        assert_eq!(read_agent_int(&store, "inst-hide", "user_hidden"), Some(0));
    }

    #[test]
    fn dual_write_instance_delete_drops_db_agents_row() {
        let store = make_store();
        let mut tpl = AgentDefinition {
            id: "tpl-instdel".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut tpl).unwrap();
        let inst = AgentInstance {
            id: "inst-del".to_string(),
            definition_id: "tpl-instdel".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: "D".to_string(),
            working_directory: "/wd/d".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        assert_eq!(count_agents(&store, "id = 'inst-del'"), 1);
        store.instance_delete("inst-del").unwrap();
        assert_eq!(count_agents(&store, "id = 'inst-del'"), 0);
    }

    #[test]
    fn dual_write_instance_repoint_updates_parent_template_id() {
        let store = make_store();
        let mut tpl_a = AgentDefinition {
            id: "tpl-A".to_string(),
            slug: String::new(),
            name: "A".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut tpl_a).unwrap();
        let mut tpl_b = tpl_a.clone();
        tpl_b.id = "tpl-B".to_string();
        tpl_b.slug = String::new();
        store.agent_def_insert(&mut tpl_b).unwrap();

        let inst = AgentInstance {
            id: "inst-rp".to_string(),
            definition_id: "tpl-A".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: "R".to_string(),
            working_directory: "/wd/r".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        assert_eq!(
            read_agent_field(&store, "inst-rp", "parent_template_id"),
            Some("tpl-A".to_string())
        );
        store.instance_repoint_definition("tpl-A", "tpl-B").unwrap();
        assert_eq!(
            read_agent_field(&store, "inst-rp", "parent_template_id"),
            Some("tpl-B".to_string())
        );
    }

    #[test]
    fn dual_write_agent_def_delete_seeded_drops_all_template_rows() {
        let store = make_store();
        for id in &["s1", "s2", "s3"] {
            let mut d = AgentDefinition {
                id: id.to_string(),
                slug: String::new(),
                name: id.to_string(),
                icon: "✦".to_string(),
                provider: "claude".to_string(),
                description: String::new(),
                working_directory: String::new(),
                shell: "bash".to_string(),
                provider_flags: String::new(),
                auto_start: 0,
                restart_on_crash: 0,
                idle_timeout_minutes: 0,
                created_at: 1000,
                agent_type: "standalone".to_string(),
                environment: String::new(),
                agent_bus_id: String::new(),
                is_seeded: 1,
                accounts: String::new(),
                parent_id: String::new(),
                branch_label: String::new(),
                updated_at: 1000,
                user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            
                memory_id: String::new(),};
            store.agent_def_insert(&mut d).unwrap();
        }
        assert_eq!(count_agents(&store, "is_template = 1"), 3);
        store.agent_def_delete_seeded().unwrap();
        assert_eq!(count_agents(&store, "is_template = 1"), 0);
    }

    /// Reagent P2 round 4 on #1013 — pins the seeded-bulk-delete
    /// scope: templates + cascaded INSTANCE projections go;
    /// user-clone DEF projections survive.
    #[test]
    fn dual_write_seeded_delete_preserves_user_clone_def_projections() {
        let store = make_store();
        // Seeded template.
        let mut tpl = AgentDefinition {
            id: "tpl-keep-check".to_string(),
            slug: String::new(),
            name: "TplCheck".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut tpl).unwrap();
        // User-clone DEF of that template (Phase 1 created this).
        let mut clone = AgentDefinition {
            id: "user-clone-keep".to_string(),
            slug: String::new(),
            name: "MaksKeeper".to_string(),
            is_seeded: 0,
            parent_id: "tpl-keep-check".to_string(),
            created_at: 1500,
            updated_at: 1500,
            ..tpl.clone()
        };
        store.agent_def_insert(&mut clone).unwrap();
        // Instance ON the seeded template (cascaded instance projection).
        let inst_on_tpl = AgentInstance {
            id: "inst-on-tpl-keep".to_string(),
            definition_id: "tpl-keep-check".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst_on_tpl).unwrap();
        // Now delete seeded → template + cascaded instance go, user-clone survives.
        store.agent_def_delete_seeded().unwrap();
        assert_eq!(count_agents(&store, "id = 'tpl-keep-check'"), 0, "template projection gone");
        assert_eq!(count_agents(&store, "id = 'inst-on-tpl-keep'"), 0, "cascaded instance projection gone");
        assert_eq!(count_agents(&store, "id = 'user-clone-keep'"), 1, "user-clone def projection survives");
    }

    /// Reagent P2 round 4 on #1013 — pins instance_update/hide/delete
    /// routing through the projection key. The previous version keyed
    /// everything on `inst.id` and silently no-op'd on folded rows.
    #[test]
    fn dual_write_instance_lifecycle_on_user_clone_def_routes_to_folded_row() {
        let store = make_store();
        // Template, user-clone def of it, instance on the user-clone.
        let mut tpl = AgentDefinition {
            id: "tpl-rt".to_string(),
            slug: String::new(),
            name: "Tpl".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),};
        store.agent_def_insert(&mut tpl).unwrap();
        let mut clone = AgentDefinition {
            id: "user-rt".to_string(),
            slug: String::new(),
            name: "Maks".to_string(),
            is_seeded: 0,
            parent_id: "tpl-rt".to_string(),
            created_at: 1500,
            updated_at: 1500,
            ..tpl.clone()
        };
        store.agent_def_insert(&mut clone).unwrap();
        let inst = AgentInstance {
            id: "inst-rt".to_string(),
            definition_id: "user-rt".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: "gh-initial".to_string(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: "id-init".to_string(),
            memory_id: "mem-init".to_string(),
            instance_name: "Maks v1".to_string(),
            working_directory: "/wd".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        // Sanity: no inst-rt row (folded).
        assert_eq!(count_agents(&store, "id = 'inst-rt'"), 0);
        assert_eq!(read_agent_field(&store, "user-rt", "github_context"), Some("gh-initial".to_string()));

        // instance_update: github_context flows through to the folded row.
        let updated = AgentInstance {
            github_context: "gh-updated".to_string(),
            ..inst.clone()
        };
        store.instance_update(&updated).unwrap();
        assert_eq!(
            read_agent_field(&store, "user-rt", "github_context"),
            Some("gh-updated".to_string()),
            "instance_update on user-clone-def routes to folded row",
        );

        // instance_set_hidden: flips user_hidden on the folded row.
        store.instance_set_hidden("inst-rt", true).unwrap();
        assert_eq!(
            read_agent_int(&store, "user-rt", "user_hidden"),
            Some(1),
            "instance_set_hidden routes to folded row",
        );

        // instance_delete: NO-OP on folded row (the def projection persists).
        store.instance_delete("inst-rt").unwrap();
        assert_eq!(
            count_agents(&store, "id = 'user-rt'"),
            1,
            "instance_delete on user-clone-def is a no-op (def projection persists)",
        );
    }
