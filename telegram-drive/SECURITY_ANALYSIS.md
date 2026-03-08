# Reanalise de Seguranca — Savedrive

Data: 2026-03-08

## Resultado executivo

Status geral: os riscos criticos originalmente apontados para a superficie ativa do app foram fechados nesta rodada.

- `api_hash` nao e persistido no banco local
- a UI ativa nao usa `innerHTML`
- o frontend legado foi removido do repositorio
- a sessao local usa segredo protegido pelo SO com migracao do esquema legado
- a CSP do app permanece explicita e restritiva

## Riscos mitigados

### 1. Segredo de sessao baseado em material previsivel

Mitigacao aplicada:
- chave raiz agora vem do keychain/secret store do sistema operacional
- blobs existentes tentam decrypt pelo esquema novo e, se necessario, pelo esquema legado
- quando o legado e aceito, o blob e regravado no esquema novo

Arquivos:
- `src/security.rs`
- `src/auth.rs`
- `src/session_store.rs`

### 2. CSP desativada

Mitigacao aplicada:
- `tauri.conf.json` e `src-tauri/tauri.conf.json` mantem `csp` e `devCsp` explicitas
- producao continua sem `unsafe-inline` e sem `unsafe-eval`

### 3. DOM-XSS por `innerHTML`

Mitigacao aplicada:
- a superficie ativa e `src/ui-app`
- o frontend legado `ui/` foi removido do repositorio
- foi adicionado gate estatico `npm run check:security-ui` para bloquear reintroducao de `innerHTML` no frontend ativo

### 4. Persistencia de `api_hash`

Mitigacao aplicada:
- `auth.prefill` persiste apenas telefone
- testes de auth verificam que `api_hash` nao reaparece serializado

## Validacao recomendada

- `cargo test`
- `npm run check:security-ui`
- `npm run test:e2e`
- `npm run build`

## Observacoes

- o segredo do SO exige que o ambiente tenha backend de credenciais funcional (Credential Manager, Keychain ou Secret Service)
- em caso de blob ou segredo invalidos, o app faz fallback seguro para `LoggedOut`, sem panic
