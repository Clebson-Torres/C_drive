# Savedrive

Desktop app em Rust + Tauri para gerenciamento de arquivos com backend Telegram, cache local, deduplicacao e UI React/Tailwind.

## Superficie ativa

- Frontend oficial: `src/ui-app/` com `Vite + React + Tailwind`
- Backend desktop: `src/`
- Configuracao Tauri: `tauri.conf.json` e `src-tauri/tauri.conf.json`

O diretório legado `ui/` foi removido e nao faz mais parte do produto.

## Status atual

- Login Telegram por telefone + codigo + 2FA
- Sessao persistente criptografada com segredo protegido pelo SO
- Upload/download paralelo com fila e cache local
- Importacao dos arquivos ja existentes em `Saved Messages`
- Frontend React/Tailwind com CSP explicita
- `api_hash` nao e persistido no banco local

## Build e execucao

```bash
npm install
npm run tauri dev
```

## Testes

### Rust
```bash
cargo test
```

### Frontend e seguranca
```bash
npm run check:security-ui
npm run test:e2e
```

### Build
```bash
npm run build
npm run build:desktop:ci
```

## Seguranca

- Sessao local protegida por segredo do SO (Windows Credential Manager / macOS Keychain / Linux Secret Service)
- Blobs legados sao migrados automaticamente no primeiro restore bem-sucedido
- CSP explicita em producao e CSP de dev restrita ao necessario para Vite/HMR
- `auth.prefill` persiste apenas telefone
- Gate estatico impede `innerHTML` no frontend ativo

## Observabilidade

- runtime log: `logs/telegram/runtime.log`
- logs de drag/drop e shell frontend usam target `savedrive_frontend`
