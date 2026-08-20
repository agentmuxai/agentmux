use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_memory_list(engine, state);
    register_memory_read(engine, state);
    register_memory_write(engine, state);
}

fn register_memory_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_MEMORY_LIST,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("memory.list: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                Ok(Some(memory_list_impl(&state, &req.agent_id)?))
            })
        }),
    );
}

fn register_memory_read(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_MEMORY_READ,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, filename: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("memory.read: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                Ok(Some(memory_read_impl(&state, &req.agent_id, &req.filename)?))
            })
        }),
    );
}

fn register_memory_write(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_MEMORY_WRITE,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, filename: String, content: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("memory.write: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                memory_write_impl(&state, &req.agent_id, &req.filename, &req.content, None)?;
                Ok(None)
            })
        }),
    );
}
