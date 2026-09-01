# AgentProf — instruções locais para Claude Code

Este é um clone do projeto público `kitaisreal/agentprof`. Preserve o estilo do
upstream e não publique, faça push ou altere configuração global sem pedido
explícito.

## Ambiente e stack

- Windows com PowerShell 7 como shell padrão. Use Bash/WSL somente quando um
  comando do upstream exigir ambiente POSIX; não misture sintaxes no mesmo comando.
- Projeto Rust. Antes de editar, leia `README.md` e `Cargo.toml`.
- Verificação mínima: `cargo fmt --check`, `cargo clippy --all-targets --all-features`
  e `cargo test`. Rode o menor teste útil durante a mudança e o conjunto completo
  somente no fechamento.

## Privacidade e hooks

- O AgentProf registra entradas e saídas completas de ferramentas. Arquivos
  `*.jsonl` podem conter código, prompts, caminhos e segredos: nunca commitar,
  publicar ou enviar esses logs a serviço externo.
- Não executar `agentprof install --global` nem modificar `~/.claude/settings.json`
  sem solicitação explícita. Testes de instalação devem usar escopo local e dados
  sintéticos.

## Comunicação e decisão

- Responder em pt-BR e explicar impacto prático de termos técnicos.
- Investigar fatos descobríveis e executar correções reversíveis já autorizadas.
  Apresente opções somente quando uma decisão do usuário realmente mudar escopo,
  custo, compatibilidade ou risco.
