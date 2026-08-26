# Kairos

Kairos es un agente personal persistente para Linux, CLI-first y con TUI nativa. Las tareas viven en SQLite, mantienen su `session_id`, eventos, estado y worktree, y usan OpenRouter como proveedor.

## Estado actual

Esta primera base implementa:

- workspace Rust con `kairos-cli`, `core`, `store`, `provider`, `runner`, `tui` y `tools`;
- configuración en `directories` y migración SQLite con SQLx;
- `task`, `status`, `logs`, `resume`, `pause`, `cancel`, `approve`, `diff`, `watch`, `cost today`, `doctor` y `config init`;
- máquina de estados persistente y eventos con tokens, cache y costo;
- cliente OpenRouter con streaming SSE y fallbacks;
- ejecución de Git encapsulada, worktrees aislados y límite de salida;
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

El modelo por defecto es `deepseek/deepseek-chat`; puede modificarse en la configuración. `resume` crea o reutiliza un worktree, ejecuta la fase de planificación vía OpenRouter, guarda uso/coste/cache y verifica el estado Git. Las herramientas con ejecución efectiva y el diálogo visual de aprobaciones quedan como el siguiente hito.

## Verificación

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
