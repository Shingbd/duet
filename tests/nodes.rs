use duet_workbench::{app::App, keymap::Keymap, nodes::*, storage::WorkspaceStore};

fn contract(id: &str, deps: &[&str]) -> NodeContract {
    NodeContract {
        id: id.into(),
        title: id.into(),
        objective: "具体目标".into(),
        scope: vec!["src".into()],
        boundaries: vec!["不改变范围外行为".into()],
        implementation: "实现方法".into(),
        constraints: vec![],
        dependencies: deps.iter().map(|s| s.to_string()).collect(),
        acceptance: vec!["验收条件".into()],
    }
}
fn plan() -> TaskPlan {
    TaskPlan {
        schema_version: 1,
        objective: "需求".into(),
        constraints: vec![],
        acceptance: vec!["总体验收".into()],
        nodes: vec![contract("second", &["first"]), contract("first", &[])],
    }
}
#[test]
fn strict_plan_parser_validates_required_fields_and_dependency_order() {
    let p = plan();
    assert_eq!(p.order().unwrap(), vec![1, 0]);
    let json = serde_json::to_string(&p).unwrap();
    assert_eq!(
        TaskPlan::parse(&format!("```json\n{json}\n```")).unwrap(),
        p
    );
    assert!(TaskPlan::parse("{\"nodes\":[]}").is_err());
    assert!(TaskPlan::parse(&format!("explanation {json}")).is_err());
    let mut raw = serde_json::to_value(&p).unwrap();
    raw["nodes"][0]
        .as_object_mut()
        .unwrap()
        .remove("implementation");
    assert!(TaskPlan::parse(&raw.to_string()).is_err());
}
#[test]
fn planning_stage_permissions_cannot_leak_into_implementation_contracts() {
    let mut p = plan();
    p.constraints = vec!["仅规划，不写入文件、不执行实现".into()];
    assert!(p.validate().is_err());
    p.constraints = vec!["产品功能读取只读数据，不改变数据库".into()];
    assert!(p.validate().is_ok());
    p.nodes[0].constraints = vec!["仅规划，不执行实现".into()];
    assert!(p.validate().is_err());
}

#[test]
fn invalid_plans_never_enter_execution() {
    for mutation in 0..6 {
        let mut p = plan();
        match mutation {
            0 => p.nodes[0].id = "first".into(),
            1 => p.nodes[0].dependencies = vec!["missing".into()],
            2 => p.nodes[1].dependencies = vec!["second".into()],
            3 => p.nodes[0].scope = vec!["../secret".into()],
            4 => p.nodes[0].acceptance.clear(),
            _ => p.nodes = (0..33).map(|i| contract(&format!("n{i}"), &[])).collect(),
        }
        assert!(p.validate().is_err(), "mutation {mutation}");
    }
    assert!(contract("n", &[]).contains_path("src/lib.rs"));
    assert!(!contract("n", &[]).contains_path("src_other/lib.rs"));
    let mut reserved = plan();
    reserved.nodes[0].scope = vec![".duet".into()];
    assert!(reserved.validate().is_err());
}
#[test]
fn execution_verification_and_approval_are_independent_and_version_bound() {
    let mut task = TaskRecord::legacy("task", "goal", "session");
    task.install_plan(plan()).unwrap();
    assert_eq!(task.next_ready(), Some(1));
    assert!(task.install_plan(plan()).is_err());
    let node = &mut task.nodes[1];
    node.begin("base");
    assert!(node.approve("cp").is_err());
    node.execution = Execution::Implemented;
    node.attempts[0].execution = Execution::Implemented;
    node.attempts[0].checkpoint = Some("cp".into());
    assert!(node.approve("wrong").is_err());
    node.approve("cp").unwrap();
    assert_eq!(node.verification, Verification::Unverified);
    node.begin("cp");
    assert_eq!(node.attempts.len(), 2);
    assert_eq!(node.review, Review::Pending);
    assert_eq!(node.reviewed_checkpoint, None);
    assert!(node.approve("cp").is_err());
}
#[test]
fn schema_three_restores_all_task_nodes_and_interrupts_without_auto_running() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = WorkspaceStore::open(dir.path()).unwrap();
    store
        .begin_run("task", "session", dir.path(), "branch", "goal")
        .unwrap();
    let mut task = TaskRecord::legacy("task", "goal", "session");
    task.install_plan(plan()).unwrap();
    task.nodes[1].begin("base");
    store.save_task(&task).unwrap();
    drop(store);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let mut app = App::empty(Keymap::default());
    store.restore(&mut app).unwrap();
    assert_eq!(app.tasks.len(), 1);
    assert_eq!(app.tasks[0].nodes.len(), 2);
    assert_eq!(app.tasks[0].state, TaskState::Interrupted);
    assert_eq!(app.tasks[0].nodes[1].execution, Execution::Interrupted);
    assert_eq!(app.tasks[0].nodes[0].execution, Execution::Planned);
    assert!(!app.agent_connected);
    let db = rusqlite::Connection::open(dir.path().join(".duet/workspace.db")).unwrap();
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        4
    );
}
#[test]
fn schema_two_migration_preserves_real_history_without_fake_node_contracts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".duet")).unwrap();
    let db = rusqlite::Connection::open(dir.path().join(".duet/workspace.db")).unwrap();
    db.execute_batch("CREATE TABLE ui_state(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE reading_progress(version TEXT,path TEXT,is_read INTEGER,scroll INTEGER,evidence_scroll INTEGER,PRIMARY KEY(version,path));
        CREATE TABLE messages(id INTEGER PRIMARY KEY,session TEXT,role TEXT,content TEXT,created_at INTEGER);
        CREATE TABLE runs(id TEXT PRIMARY KEY,session TEXT,worktree TEXT,branch TEXT,prompt TEXT,status TEXT,checkpoint TEXT,created_at INTEGER);
        INSERT INTO runs VALUES('old','session','tree','branch','真实历史需求','Settled',NULL,1);
        INSERT INTO messages VALUES(1,'session','user','真实历史需求',1); PRAGMA user_version=2;").unwrap();
    drop(db);
    let store = WorkspaceStore::open(dir.path()).unwrap();
    let tasks = store.tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].nodes.is_empty());
    assert!(tasks[0].plan.is_none());
    assert_eq!(tasks[0].state, TaskState::Implemented);
    assert_eq!(tasks[0].history[0].content, "真实历史需求");
}
