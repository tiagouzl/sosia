# sosia — File Deduplication Engine

Motor de deduplicação de arquivos em Rust: varre um diretório, identifica
conteúdo idêntico com **BLAKE3** e reporta o espaço recuperável — sem apagar
nada sem sua confirmação explícita.

```
sosia scan ~/Downloads          # relatório humano
sosia scan --json ~/Downloads   # manifesto JSON (para find/verify)
```

## Como funciona

O pipeline em quatro estágios (mais um estágio 0 de contagem) minimiza o I/O
real — a maior parte do tempo de um verificador ingênuo é gasta lendo bytes que
não precisam ser lidos:

| Estágio | O que faz | Custo de I/O |
|---|---|---|
| 0. Varredura | `walkdir` sequencial indexa tamanho, mtime e `(device, inode)` | só metadados |
| 1. Tamanho | agrupa por tamanho; tamanhos únicos nunca são lidos | **zero leitura de conteúdo** |
| 2. Hash parcial | BLAKE3 dos primeiros 4 KiB em paralelo (Rayon); prefixos únicos saem | 4 KiB por candidato |
| 3. Hash completo | BLAKE3 *streaming* apenas de candidatos que sobreviveram | arquivo inteiro |
| 4. Saída | grupos ordenados por espaço recuperável, manifesto JSON | — |

A métrica de I/O é auditável: `bytes_hashed` no manifesto conta exatamente os
bytes lidos nos estágios de hash. `sosia verify` re-abre cada arquivo citado e
re-hash para provar que o relatório ainda descreve o disco.

## Comandos

```
sosia scan <CAMINHO> [--json] [--output ARQ] [--include-empty] [--include-hidden]
sosia find <MANIFESTO> <HASH>      # hash completo ou prefixo de 8+ hex
sosia verify <MANIFESTO>           # código de saída 1 se algo mudou
sosia interactive <CAMINHO> [--apply] [--include-hidden]
```

### Segurança primeiro

`interactive` **simula por padrão**: nada é apagado sem `--apply`, e cada grupo
pede confirmação com opção de pular. Não existe `--dry-run` — a simulação é o
comportamento padrão e a remoção é a exceção opt-in. Erros por arquivo (permissão,
arquivo já removido) são registrados e não abortam a sessão.

Códigos de saída: `0` sucesso · `1` sem correspondência (`find`) ou problemas de
integridade (`verify`) · `2` erro de execução.

### Comportamentos deliberados

* **Hard links** (`st_dev`, `st_ino`) são indexados uma vez por inode: apagar
  um caminho do par não devolve espaço, então não conta como desperdício.
* **Symlinks não são seguidos** (evita ciclos e dupla contagem).
* **Ocultos** (`.`) são podados por padrão — inclusive diretórios inteiros;
  `--include-hidden` reverte. A raiz indicada é sempre varrida, mesmo oculta.
* **Arquivos vazios** ficam fora por padrão (apagar duplicatas vazias não
  recupera espaço); `--include-empty` os traz de volta.
* **Erros de varredura** (permissão etc.) são contados em `walk_errors` e a
  varredura continua.

## Benchmark (medido neste host)

Nenhum número abaixo é estimativa: tudo vem de `examples/benchmark.rs`
(`cargo run --release --example benchmark`), que executa o pipeline e uma linha
de base ingênua (hash BLAKE3 completo de todos os arquivos) **sobre o mesmo
dataset sintético**, com as páginas liberadas do cache do kernel entre as rodadas
(`posix_fadvise(POSIX_FADV_DONTNEED)`), e depois **compara os grupos produzidos
por ambos — divergência é um erro de execução**, garantindo que a otimização não
troca corretude por velocidade.

* **Host:** Intel i5-10210U (8 threads lógicas), ~7,6 GiB de RAM, NVMe em ext4;
  dataset em `/tmp` (disco real, não tmpfs). Executado em 4 rodadas em 18/09/2026.
* **Dataset:** 400 arquivos, 640,21 MB — 160 arquivos de tamanho único
  (o pipeline nunca lê o conteúdo deles) + 80 grupos de duplicatas com 3 cópias
  de 2 MiB cada.

| Estratégia | Tempo (cache frio) | Bytes lidos | Vazão | Grupos |
|---|---|---|---|---|
| Linha de base ingênua (hash de tudo) | 524–559 ms | 640,21 MB | ~1146–1223 MiB/s | 80 |
| Pipeline (tamanho → 4 KiB → BLAKE3) | 46–95 ms | 480,94 MB | 5056–10359 MiB/s | 80 |
| **Speedup (4 rodadas, mediana)** | **~9,2×** (faixa 5,5–11,5×) | −24,9% de leitura | | idênticos |

A leitura evitada (159,27 MB) é exatamente o conteúdo dos 160 arquivos de
tamanho único, descartados no estágio 1 sem I/O — e a equivalência dos grupos
foi verificada em todas as execuções. A variação do speedup vem do lado do
pipeline (46–95 ms); a linha de base é estável, pois é limitada por I/O.


## Trade-offs conhecidos

* **Varredura sequencial**: a varredura é limitada por metadados/syscalls;
  paralelizá-la costuma trocar CPU por *thrashing* de disco. O paralelismo fica
  nas etapas de hash (CPU-bound), onde escala de verdade.
* **Hash completo é serializado por grupo de prefixo**: candidatos que dividem o
  mesmo prefixo de 4 KiB são hasheados juntos para não ler o mesmo arquivo duas
  vezes; em diretórios com milhões de colisões de prefixo reais, isso limita o
  paralelismo dessa etapa.
* **`--threads N`**: Rayon é configurado globalmente no início do processo; o
  limite se aplica a todos os estágios paralelos.

## Limites

* O manifesto (`scan --json --output`) é uma fotografia: arquivos podem mudar
  depois da varredura — por isso `sosia verify` existe e é barato de rodar.
* `format_version` no JSON (atualmente `1`) permite evoluir o formato com aviso
  explícito em vez de falha silenciosa em `find`/`verify`.

## Desenvolvimento

```
cargo test              # 37 testes (unitários + integração)
cargo clippy --all-targets
cargo run --release --example benchmark
```

Licença: MIT.

