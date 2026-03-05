# Telegram Drive

Desktop app em Rust + Tauri para gerenciamento de arquivos com backend Telegram, com cache local, deduplicação, upload/download paralelo e UI de file manager.

## Status atual

- Login Telegram por telefone + código + 2FA
- Persistência de sessão local criptografada (sem relogin após reinício)
- Fluxo QR removido do produto
- Tela de login dedicada (multietapas)
- Testes unitários + integração + E2E mockado

## Arquitetura

- **UI (`ui/`)**: login, navegação de arquivos, busca, progresso e preview.
- **Core (`src/`)**:
  - `auth.rs`: orquestra login, restauração e persistência de sessão de auth.
  - `telegram.rs`: cliente Telegram, autenticação e transporte de chunks.
  - `session_store.rs`: `PersistentSession` (`grammers_session::Session`) com arquivo criptografado + debounce + escrita atômica.
  - `database.rs`, `dedup.rs`, `chunking.rs`, `cache.rs`, `uploader.rs`, `downloader.rs`, `file_index.rs`, `progress.rs`.

## Build e execução

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

### E2E (Playwright, mock CI)
```bash
npm install
npm run test:e2e
```

Arquivos E2E:
- `e2e/auth_flow.spec.ts`
- `playwright.config.ts`

### Smoke desktop (manual)
- roteiro: `scripts/desktop-smoke-tauri-driver.md`

## Segurança

- Não hardcode de credenciais no código.
- Login recebe `phone`, `api_id`, `api_hash` pela tela.
- Dados sensíveis de sessão persistidos localmente em formato criptografado.

## Observabilidade

Spans de tracing para:
- `auth_start`
- `auth_verify_code`
- `auth_verify_password`
- `session_restore`
