# Desktop Smoke (tauri-driver)

Este smoke é manual e valida login real no app desktop.

## Pré-requisitos
- `tauri-driver` instalado
- `cargo run` funcionando
- Credenciais válidas no formulário de login

## Passos
1. Inicie o app com logs:
   - `RUST_LOG=info cargo run`
2. Na tela de login, preencha telefone/API ID/API hash.
3. Valide código Telegram.
4. Se houver 2FA, valide senha.
5. Feche e reabra o app.
6. Confirme que abre já autenticado (`auth_status = LoggedIn`) sem pedir novo código.

## Verificações
- Sem crash/panic de sessão
- Sem botão/fluxo QR
- File manager disponível após login
- Persistência mantida entre reinícios
