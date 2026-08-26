# Kairos

Kairos es un agente personal persistente para Linux, CLI-first y con TUI nativa. Las conversaciones y ejecuciones viven en SQLite; cada conversación mantiene su `session_id`, historial, eventos, estado y worktree cuando hace falta, y usa OpenRouter como proveedor.

## Estado actual

Esta primera base implementa:

- workspace Rust con `kairos-cli`, `core`, `store`, `provider`, `runner`, `tui` y `tools`;
- configuración en `directories` y migración SQLite con SQLx;
- `task`, `status`, `logs`, `resume`, `pause`, `cancel`, `approve`, `diff`, `watch`, `cost today`, `doctor` y `config init`;
- máquina de estados persistente y eventos con tokens, cache y costo;
- cliente OpenRouter con streaming SSE y fallbacks;
- ejecución de Git encapsulada, worktrees aislados y límite de salida;
- conversaciones por repositorio con historial persistente de mensajes y `session_id` estable;
- TUI inicial con panel de tareas, foco por ID, navegación `j/k` y refresco periódico.
- Acciones TUI conectadas: `n` crea, `r` reanuda, `p` pausa, `a` muestra aprobación, `d` abre diff, `l` abre logs y `c` muestra costos.

## Uso rápido

```bash
export OPENROUTER_API_KEY=...
cargo run -- config init
cargo run -- task "diagnostica el backend" --repo ~/code/app --detach
cargo run -- status
cargo run -- logs <task-id>
cargo run -- resume <task-id>
cargo run -- watch
cargo run -- cost today
```

Dentro de la TUI: `j/k` navega, `Tab` cambia el foco, `Enter` abre una tarea, `/` busca, `?` muestra ayuda y `Esc`/`q` vuelve o sale. Las aprobaciones se confirman con `y`/`Enter` o se rechazan con `n`/`Esc`.

El modelo por defecto es `deepseek/deepseek-chat`; puede modificarse en la configuración. Los prompts enviados desde un mismo repositorio reutilizan la conversación y su historial reciente. `resume` crea o reutiliza un worktree, ejecuta la fase de planificación vía OpenRouter, guarda uso/coste/cache y verifica el estado Git. La TUI chat-first y el bucle de herramientas efectivo quedan como los siguientes hitos.

## Verificación

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
