# Bufonidae — plano

App desktop Linux (Rust) que agrega N contas Mega.nz num diretório lógico único e distribui arquivos entre elas, usando `rclone` como motor de transferência.

## 1. Arquitetura

```
bufonidae/                      (Cargo workspace, edition 2024, toolchain pinada)
├─ crates/
│  ├─ bufonidae-core/           placement, índice, journal, quotas, políticas (sem I/O de UI)
│  ├─ bufonidae-rclone/         cliente do `rclone rcd` (JSON-RPC/HTTP), supervisor do processo
│  └─ bufonidae-cli/            binário `bufonidae` (clap), mesma core
├─ apps/
│  └─ desktop/                  gpui + gpui-component, mesma core
├─ website/                     crate Rust + Topcoat (SSR), Dockerfile + compose.yml
└─ docs/
```

Regra: core não depende de gpui nem de clap. CLI e desktop são finas; toda lógica testável vive em core.

Fluxo: `drop` (GUI/CLI/watch folder) → **planner** (core) decide conta-destino por item → **journal** (SQLite) grava intenção → `rclone rcd` executa (`operations/copyfile`, `sync/copy`) → verificação → journal confirma → índice atualiza.

## 2. Problemas e soluções

| # | Problema | Solução |
|---|----------|---------|
| 1 | **ToS do Mega** proíbe burlar limites de quota; várias contas gratuitas podem ser banidas. | Não automatizar criação de contas. Só conectar contas que o usuário já tem. Aviso explícito no primeiro uso. Documentar no README/website. |
| 2 | **rsync não fala Mega.** | rsync não entra no caminho Mega. Uso legítimo: staging local (`--partial --checksum`) de pasta observada → área temporária, e espelho local→local/NAS. Se não houver caso real no MVP, adiar. Dizer isso na doc, sem inflar. |
| 3 | **Mega não tem hash/mtime utilizáveis no rclone** (backend `mega`): `--checksum` não funciona, comparação é por tamanho. | Calcular hash local (blake3) no upload e guardar no índice; verificar integridade por tamanho + hash do índice; `rclone check --size-only` como fallback. Verificar versão do rclone ao pinar. |
| 4 | **Cota de transferência (bandwidth) por conta**, além do espaço. Download falha ao estourar. | Modelar duas cotas por conta: espaço livre (`operations/about`) e orçamento de banda estimado com janela móvel. Leitura escolhe a réplica/conta com banda sobrando; backoff em erro de quota. |
| 5 | **Mover entre contas = download + upload** (sem move server-side entre contas). Rebalancear é caro. | Acertar o placement na primeira vez. Rebalanceamento só manual/agendado, com estimativa de custo mostrada antes. |
| 6 | **Placement ruim**: arquivos grandes, pastas com muitos arquivos pequenos, pastas que devem ficar juntas. | Políticas plugáveis (trait `Placement`): `most-free-space` (padrão), `round-robin`, `keep-folder-together` (pasta inteira numa conta se couber), `spread` (por arquivo). Bin packing first-fit-decreasing quando o drop é um lote. Folga mínima configurável por conta (ex. 5%). |
| 7 | **Arquivo maior que o espaço livre de qualquer conta.** | Chunking opcional: partes de N MB em contas distintas + manifesto no índice. Fase 2; MVP rejeita com erro claro. Alternativa: `rclone chunker` sobre `union` (avaliar). |
| 8 | **Perder uma conta = perder parte dos dados** (sem redundância). | Fator de réplica por pasta (1 padrão, 2 opcional). Mostrar no UI "dados em risco" por conta. Índice exportável. |
| 9 | **Índice diverge da realidade** (alguém mexeu pelo site do Mega, crash no meio). | Fonte da verdade = listagem remota; índice é cache. Journal write-ahead para recovery. Comando/ação `reconcile` (`operations/list` recursivo) detecta órfãos, faltantes, duplicados. |
| 10 | **Mega permite nomes duplicados** na mesma pasta. | Planner nunca cria duplicata; reconcile sinaliza (`rclone dedupe` só com confirmação). |
| 11 | **Rate limit / login** (muitos logins simultâneos → erros `-3`/`-9`, bloqueio temporário). | Uma instância `rcd` única gerencia as N contas; sessões reaproveitadas; semáforo de concorrência por conta e global; retry com backoff exponencial + jitter. |
| 12 | **Credenciais**: `rclone.conf` guarda senha só ofuscada (obscure ≠ criptografia). | Segredos no Secret Service (GNOME Keyring/KWallet) via `keyring`/`oo7`. Config do rclone gerada em runtime (arquivo temp 0600 em `XDG_RUNTIME_DIR` ou `config/create` via RC) e nunca persistida com senha. 2FA: confirmar suporte na versão do rclone e expor campo no UI. |
| 13 | **Privacidade do metadado**: índice local revela estrutura. | SQLite em `XDG_DATA_HOME`, permissão 0600; opção SQLCipher fora do MVP. Mega já é E2EE; `crypt` opcional por cima fica fora do MVP. |
| 14 | **Dependência de binário externo** (rclone ausente/versão errada). | Detectar na inicialização (`rclone version`), versão mínima pinada, mensagem acionável. Não embutir no MVP; empacotamento (AppImage/Flatpak) pode incluir. |
| 15 | **Processo `rcd` órfão / porta exposta.** | Subir com `--rc-addr 127.0.0.1:0` (porta efêmera) + `--rc-user/--rc-pass` aleatórios por sessão; supervisor mata no drop; `kill_on_drop`. |
| 16 | **gpui é instável** (API muda sem semver estrito; ecossistema em fluxo). | Pin exato (rev git do zed ou versão do crates.io, confirmar o que está publicado na hora do scaffold). Camada fina de views isolada em `apps/desktop`; atualização de gpui = commit próprio. `gpui-component` para widgets (tabela, tree, input). |
| 17 | **gpui no Linux exige libs de sistema** (Vulkan, Wayland/X11, xkbcommon, fontconfig…) e GPU. | Lista no README e no CI (apt). Verificar fallback de software rendering; documentar requisitos. |
| 18 | **Testar sem Mega** (conta real em CI = segredo + flaky + ToS). | Contas fake = backend `local` ou `memory` do rclone com limites simulados. Trait `RemoteFs` em core, impl `rclone` e impl `fake` determinística. Testes live contra Mega só manuais/`--ignored`, fora do CI. |
| 19 | **Operações longas na UI** (cancelar, retomar, progresso). | Fila de jobs em core com estados persistidos; progresso via `core/stats`/`job/status` do rcd; UI assina canal de eventos. Retomável após crash pelo journal. |
| 20 | **Segurança de caminhos**: drop de symlinks, `..`, nomes inválidos, arquivos mudando durante upload. | Canonicalizar, política de symlink explícita (não seguir por padrão), snapshot de size/mtime antes e checagem depois. |

## 3. Modelo de dados (SQLite, `rusqlite` bundled)

- `account(id, label, remote_name, quota_total, quota_used, bw_window…, enabled)`
- `item(id, virtual_path, size, blake3, kind)`; `placement(item_id, account_id, remote_path, state)`
- `journal(id, op, payload, state, ts)` — write-ahead
- Diretório virtual = conjunto de contas + política; "3 contas no mesmo diretório" = um *pool*.

## 4. Fases

1. **MVP-0**: workspace, CI, `bufonidae-rclone` (spawn rcd, list/about/copy), CLI `account add/list`, `put <path> --pool X`. Fake backend + testes de planner.
2. **MVP-1**: journal, reconcile, políticas, cotas, `get`/`ls` unificado, TUI-free CLI estável.
3. **MVP-2**: desktop gpui (contas, pools, drag&drop, fila, saúde das contas).
4. **Fase 3**: réplica, chunking, watch folder (+ rsync staging se houver caso), empacotamento, website.

## 5. Dependências

**Sistema**: `rclone` (versão mínima pinada após testar o backend mega), `rsync` (opcional), Secret Service (gnome-keyring/kwallet), libs gpui: `libxkbcommon`, `wayland`, `libxcb`, `vulkan` loader, `fontconfig`, `freetype`. Dev: `gh`, `docker`.

**Rust, core/rclone/cli** (versões resolvidas com `cargo add`/`cargo info` no scaffold e congeladas no `Cargo.lock`; deps compartilhadas em `[workspace.dependencies]`):
- `tokio` (rt, process, sync, time) — async, supervisor do rcd
- `reqwest` (rustls, json; sem openssl) — cliente RC
- `serde`, `serde_json`, `toml` — protocolo e config
- `rusqlite` (bundled) — índice/journal
- `thiserror` (libs), `anyhow` (só binários)
- `tracing`, `tracing-subscriber` — logs
- `clap` (derive) — CLI
- `directories` — caminhos XDG
- `keyring` ou `oo7` — segredos
- `blake3` — hash local
- `notify` — watch folder (fase 3)
- `uuid`, `bytesize`, `humantime`
- dev: `tempfile`, `proptest` (planner), `insta` (snapshots), `wiremock`/`axum` fake rcd, `assert_cmd`

**Desktop**: `gpui`, `gpui-component` (pins coordenados), `rfd` ou diálogo via gpui para seleção de pasta, `ashpd` se precisar portal XDG.

**Website**: Topcoat (SSR), fatos lidos dos arquivos do produto em build, `Dockerfile` multi-stage + `compose.yml`.

**Tooling**: `rustfmt`, `clippy`, `cargo-deny` (licenças/advisories/bans), `cargo-audit`, `cargo-machete`.

## 6. Boas práticas

- `rust-toolchain.toml` pinado (1.99.1 hoje), `rustfmt.toml`, `[workspace.lints]` (`unsafe_code = "forbid"` em core, clippy `pedantic` seletivo).
- `[workspace.package]` herdado (edition, license, rust-version).
- Errors tipados em libs; `anyhow` só na borda. Sem `unwrap` fora de testes.
- Core síncrono onde possível; async só na fronteira de I/O. Traits (`RemoteFs`, `Placement`, `Clock`) para teste determinístico.
- Perfis: `dev debug=0`, deps dev `debug=false`; `release strip=true, lto="thin", codegen-units=16`; `[profile.website]` size-tuned.
- Commits: gerados com `pionus`, revisados contra o diff; commits de bump de dependência/toolchain separados, com motivo.
- Nenhum segredo em log; redigir senhas/tokens em `Debug`.
- Licença e `SECURITY.md`; aviso de ToS no README.

## 7. CI/CD (conforme regras do ai-memory)

`.github/workflows/ci.yml`:
- `concurrency: ci-${{ github.workflow }}-${{ github.ref }}`, `cancel-in-progress: true`
- `permissions: contents: read`; `env: CARGO_TERM_COLOR=always, CARGO_INCREMENTAL=0, CARGO_NET_RETRY=10`
- `Swatinem/rust-cache@v2` pinado por SHA, mesmo `shared-key` em todos os jobs
- `--locked` em todo cargo; **um** `cargo fmt --check`, **um** `cargo clippy --workspace --all-targets -- -D warnings`, **um** `cargo test --workspace`
- apt das libs gpui no job
- Ferramentas (`cargo-deny`, `cargo-audit`) cacheadas por versão, `cargo install --version X --locked` só em cache miss
- Actions de terceiros pinadas por commit SHA

Outros workflows:
- `audit.yml` — semanal: `cargo audit`, `cargo deny check`, relatório de toolchain/ferramentas defasadas (abre issue)
- `dependabot.yml` — cargo + github-actions (semanal)
- `release.yml` — em tag `v*`, sem cancel-in-progress: build release do `cli` e `desktop`, tarball + `.deb`/AppImage, checksums, GitHub Release
- `website.yml` — build da imagem Docker (sem push no PR)

## 8. Repositório

`gh repo create` em `Lucas-BRT/bufonidae`, branch padrão `main` (local está em `master`, sem commits → renomear), `.gitignore` (já existe), LICENSE, README, `AGENTS.md`/`CLAUDE.md`.

## 9. Decisões

- Repo privado inicialmente, licença MIT.
- Redundância: opcional e configurável por pool, modelo RAID. Níveis: `single` (JBOD, padrão), `mirror` (RAID1, N cópias em contas distintas), `parity` (estilo RAID5/6 via erasure coding, k+m partes) como evolução. O nível é propriedade do pool (e sobrescrevível por pasta); o planner escolhe N contas distintas por item. `mirror` entra no MVP-1; `parity` depois.

## 10. Em aberto

- Granularidade padrão do placement (pasta inteira vs arquivo a arquivo).
- Caso de uso do rsync (staging local / espelho NAS) ou adiar.
