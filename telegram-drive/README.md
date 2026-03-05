# Telegram Drive

Desktop app em Rust + Tauri para gerenciamento de arquivos com backend Telegram, com cache local, deduplicacao, upload/download paralelo e UI de file manager.

## Status atual

- Login Telegram por telefone + codigo + 2FA
- Persistencia de sessao local criptografada (sem relogin apos reinicio)
- Fluxo QR removido do produto
- Tela de login dedicada (multietapas)
- Upload hibrido:
  - ate `1 GiB`: objeto unico original no Telegram
  - acima de `1 GiB`: chunking de `8 MiB` com criptografia + dedup por chunk
- Testes unitarios + integracao + E2E mockado

## Arquitetura

- **UI (`ui/`)**: login, navegacao de arquivos, busca, progresso e preview.
- **Core (`src/`)**:
  - `auth.rs`: orquestra login, restauracao e persistencia de sessao de auth.
  - `telegram.rs`: cliente Telegram, autenticacao e transporte de objetos/chunks.
  - `session_store.rs`: `PersistentSession` (`grammers_session::Session`) com arquivo criptografado + debounce + escrita atomica.
  - `database.rs`, `dedup.rs`, `chunking.rs`, `cache.rs`, `uploader.rs`, `downloader.rs`, `file_index.rs`, `progress.rs`.

## Build e execucao

```bash
cargo check
cargo test
cargo run
```

## Testes

### Rust
```bash
cargo test
```

### Matriz real de transferencias
Executa upload/download com arquivos fisicos reais em backend mockado do Telegram, gerando logs ricos de throughput e modo de armazenamento:

```bash
cargo test upload_download_matrix_real_files -- --ignored --nocapture
```

Artefatos:
- `logs/manual/transfer-matrix-run.log`
- `logs/perf/transfer-matrix.jsonl`
- `logs/perf/transfer-matrix-summary.md`

### E2E (Playwright, mock CI)
```bash
npm install
npm run test:e2e
```

Arquivos E2E:
- `e2e/auth_flow.spec.ts`
- `e2e/click_regression.spec.ts`
- `playwright.config.ts`

### Smoke desktop (manual)
- roteiro: `scripts/desktop-smoke-tauri-driver.md`

## Seguranca

- Nao hardcode de credenciais no codigo.
- Login recebe `phone`, `api_id`, `api_hash` pela tela.
- Dados sensiveis de sessao persistidos localmente em formato criptografado.

## Observabilidade

Spans de tracing para:
- `auth_start`
- `auth_verify_code`
- `auth_verify_password`
- `session_restore`
- `upload_file`
- `download_file`

Runtime log:
- `logs/telegram/runtime.log`
