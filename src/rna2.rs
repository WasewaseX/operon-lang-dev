//! rna2.rs, W067 stage 2: the node-addressed `.rna` edit engine.
//!
//! V1 (genes::apply_rna_checked, W068) edits TEXT SPANS: `edit target {
//! replace "src" -> "dst"; }`, fragile under reformatting and
//! substring-precise. V2 (this module) edits AST NODES: the current source
//! is parsed fresh ("parse, don't slice"), rules address declarations by
//! name+ordinal ("node addressing"), and the edited AST is REPRINTED with
//! the canonical formatter ("reprint, don't splice"), so the output of an
//! apply is fmt-stable by construction (fmt∘fmt = fmt is enforced
//! corpus-wide, W047).
//!
//! Patch self-description: a v2 patch's first non-blank, non-comment line
//! MUST be exactly `syntax: v2`. Absent header → v1 text semantics
//! (byte-compatible; the same CLI, `operon rna f.op patch.rna`, dispatches).
//!
//! v2 verbs (deliberately small):
//!   rename <target> -> <new-name>   decl name + intra-file reference nodes
//!   delete <target>                 decl removed; references fail honestly
//!   body <gene> { ... }             whole-body replacement, parsed FIRST
//!
//! v2 target paths:
//!   gene <name>[#<ordinal>]         ordinal disambiguates same-name decls
//!   splice <root> · variant <root>.<name>
//!   phenotype <name> · method <pheno>.<gene> · fate <name>
//!   regulate #<ordinal>             1-indexed statement order (delete only)
//!
//! Safety contract (inherits W068, tightens it):
//!   * ALL-OR-NOTHING: every rule is resolved against the (sequentially
//!     edited) AST; ANY miss → nothing is written, per-rule fate reported,
//!     exit 1.
//!   * Bare-name ambiguity REFUSES: two declarations named `foo` require
//!     `foo#1` / `foo#2` (v1 replaced substrings blindly).
//!   * COMMENT PREFLIGHT: the reprint drops plain `#` comments (only `##`
//!     doc comments attached to declarations roundtrip, W074). A source
//!     containing plain comments is REFUSED unless `--allow-comment-drop`.
//!   * The patch grammar is a FILE FORMAT parsed here, the language
//!     surface never changes (bio-layer freeze, D-011/R9): `edit/replace`
//!     stays in the grammar, this module never touches the shared
//!     lexer/parser.
//!
//! Rename semantics (v2 scope, documented): `rename gene` rewrites the
//! declaration name AND every `Ident` reference node in the file,
//! identifier-precise, but NOT scope-aware (a shadowing local of the same
//! name is rewritten too; strictly safer than v1's substring replace).
//! `rename phenotype` rewrites `New(...)` constructor nodes and type
//! annotations; `rename method` rewrites `Method(...)/MethodSafe(...)`
//! name strings (name-global, pick unique method names);
//! `rename splice` rewrites the root binding's identifier references;
//! `rename fate` rewrites `FateNew(...)` nodes. `.cell`/CLI keys (variant
//! selection, methylation targets) are configuration, not source, a patch
//! edits source only.

use crate::ast::*;
use crate::tools;
use std::sync::Arc;

// ------------------------------------------------------------ patch model

/// A v2 rule's target, after patch parsing.
#[derive(Debug, Clone)]
pub enum V2Target {
    Gene {
        name: String,
        ordinal: Option<usize>,
    },
    Splice {
        root: String,
    },
    Variant {
        root: String,
        name: String,
    },
    Pheno {
        name: String,
    },
    Method {
        pheno: String,
        gene: String,
    },
    Fate {
        name: String,
    },
    Regulate {
        ordinal: usize,
    },
}

impl V2Target {
    fn canonical(&self) -> String {
        match self {
            V2Target::Gene { name, ordinal } => match ordinal {
                Some(n) => format!("gene {}#{}", name, n),
                None => format!("gene {}", name),
            },
            V2Target::Splice { root } => format!("splice {}", root),
            V2Target::Variant { root, name } => format!("variant {}.{}", root, name),
            V2Target::Pheno { name } => format!("phenotype {}", name),
            V2Target::Method { pheno, gene } => format!("method {}.{}", pheno, gene),
            V2Target::Fate { name } => format!("fate {}", name),
            V2Target::Regulate { ordinal } => format!("regulate #{}", ordinal),
        }
    }
}

/// A v2 rule's verb.
#[derive(Debug, Clone)]
pub enum V2Verb {
    Rename(String),
    Delete,
    /// Whole-body replacement source (raw text, parsed at apply time, a
    /// parse error in the replacement refuses the whole apply).
    Body(String),
}

#[derive(Debug, Clone)]
pub struct V2Rule {
    pub verb: V2Verb,
    pub target: V2Target,
}

// ------------------------------------------------------------ report

/// One rule's fate under the v2 engine (W068 fate-report contract,
/// node-flavored).
#[derive(Debug, Clone)]
pub struct Rna2RuleReport {
    pub verb: String,
    pub target: String,
    /// the target node existed (and was unambiguous)
    pub target_found: bool,
    /// found AND mutated (rename to the same name is found but no-op)
    pub applied: bool,
    pub detail: String,
}

/// Result of a v2 apply. `new_text` is None when the all-or-nothing
/// contract refused the apply (any rule missed), nothing would be written.
#[derive(Debug, Clone)]
pub struct Rna2Report {
    pub rules: Vec<Rna2RuleReport>,
    pub new_text: Option<String>,
}

impl Rna2Report {
    pub fn applied(&self) -> usize {
        self.rules.iter().filter(|r| r.applied).count()
    }
    pub fn missed(&self) -> usize {
        self.rules.iter().filter(|r| !r.target_found).count()
    }
    pub fn would_change(&self) -> bool {
        self.new_text.is_some() && self.applied() > 0
    }
}

// ------------------------------------------------------------ detection

/// True when the patch's first non-blank, non-comment line is exactly
/// `syntax: v2`. Anything else (including an empty patch) is v1.
pub fn is_v2_patch(patch_src: &str) -> bool {
    for line in patch_src.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        return t == "syntax: v2";
    }
    false
}

// ------------------------------------------------------------ comment guard

/// 1-indexed line numbers of PLAIN `#` comments (single `#`, not `##`) that
/// live outside string literals. `"""` multiline strings are tracked across
/// lines; `##` lines are doc comments and NOT flagged (attached ones
/// roundtrip via W074). Heuristic safety net: errs toward refusal.
pub fn plain_comment_lines(src: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut in_triple = false;
    for (n, line) in src.lines().enumerate() {
        if scan_line_plain_hash(line, &mut in_triple) {
            out.push(n + 1);
        }
    }
    out
}

fn scan_line_plain_hash(line: &str, in_triple: &mut bool) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let n = chars.len();
    let mut in_str = false;
    while i < n {
        if *in_triple {
            if chars[i] == '"' && i + 2 < n && chars[i + 1] == '"' && chars[i + 2] == '"' {
                *in_triple = false;
                i += 3;
            } else {
                i += 1;
            }
            continue;
        }
        let c = chars[i];
        if in_str {
            match c {
                '\\' => i += 2,
                '"' => {
                    in_str = false;
                    i += 1;
                }
                _ => i += 1,
            }
            continue;
        }
        match c {
            '"' => {
                if i + 2 < n && chars[i + 1] == '"' && chars[i + 2] == '"' {
                    *in_triple = true;
                    i += 3;
                } else {
                    in_str = true;
                    i += 1;
                }
            }
            '#' => {
                // `##` doc comment, attached ones roundtrip (W074); not flagged
                if i + 1 < n && chars[i + 1] == '#' {
                    return false;
                }
                return true;
            }
            _ => i += 1,
        }
    }
    false
}

/// The comment-preflight refusal message. Shared VERBATIM by the apply path
/// (`apply_rna_v2`) and check mode (W68) so the two contracts cannot drift.
pub fn comment_refusal_msg(comment_lines: &[usize]) -> String {
    format!(
        "rna v2: refused, plain '#' comments at lines [{}] would be lost by the AST reprint; \
         convert them to '##' doc comments or pass --allow-comment-drop",
        comment_lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// ------------------------------------------------------------ patch parser

/// Parse a v2 patch (line-based FILE format, not language syntax).
pub fn parse_v2_patch(src: &str) -> Result<Vec<V2Rule>, String> {
    let mut rules = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut seen_header = false;
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        i += 1; // i is now the 1-indexed line number of `t`
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if !seen_header {
            if t == "syntax: v2" {
                seen_header = true;
                continue;
            }
            return Err(format!(
                "patch parse error at line {}: expected 'syntax: v2' header first, found '{}'",
                i, t
            ));
        }
        let toks: Vec<&str> = t.split_whitespace().collect();
        match toks[0] {
            "rename" | "delete" => {
                let is_rename = toks[0] == "rename";
                if toks.len() < 3 {
                    return Err(format!(
                        "patch parse error at line {}: '{} needs a target'",
                        i, toks[0]
                    ));
                }
                let target = parse_target(&toks[1..], is_rename, i)?;
                let verb = if is_rename {
                    // rename <target> -> <new>: arrow + new name are the last two tokens
                    if toks.len() < 5 || toks[toks.len() - 2] != "->" {
                        return Err(format!(
                            "patch parse error at line {}: rename needs '-> <new-name>'",
                            i
                        ));
                    }
                    let new_name = toks[toks.len() - 1];
                    if new_name.contains('#') {
                        return Err(format!(
                            "patch parse error at line {}: new name '{}' must not contain '#'",
                            i, new_name
                        ));
                    }
                    V2Verb::Rename(new_name.to_string())
                } else {
                    V2Verb::Delete
                };
                rules.push(V2Rule { verb, target });
            }
            "body" => {
                // body gene NAME[#N] {  … verbatim lines …  }
                if toks.len() < 3 || toks[1] != "gene" {
                    return Err(format!(
                        "patch parse error at line {}: body targets a gene: 'body gene NAME[#N] {{'",
                        i
                    ));
                }
                let last = toks[toks.len() - 1];
                if last != "{" {
                    return Err(format!(
                        "patch parse error at line {}: body opens with '{{' as the last token",
                        i
                    ));
                }
                let name_tok = toks[2];
                let (name, ordinal) = parse_name_ordinal(name_tok).ok_or_else(|| {
                    format!("patch parse error at line {}: bad gene '{}'", i, name_tok)
                })?;
                // gather verbatim body lines until the matching close brace
                let mut content = String::new();
                let mut depth: i64 = 1; // the opening '{'
                let mut in_str = false;
                let mut closed = false;
                while i < lines.len() {
                    let bl = lines[i];
                    i += 1;
                    let (line_out, d) = scan_body_line(bl, depth, in_str, &mut closed);
                    depth = d;
                    in_str = body_str_state(bl, in_str);
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&line_out);
                    if closed {
                        break;
                    }
                }
                if !closed {
                    return Err(format!(
                        "patch parse error: 'body gene {}' opened at line {} is never closed",
                        name, i
                    ));
                }
                rules.push(V2Rule {
                    verb: V2Verb::Body(content),
                    target: V2Target::Gene { name, ordinal },
                });
            }
            other => {
                return Err(format!(
                    "patch parse error at line {}: unknown verb '{}' (rename | delete | body)",
                    i, other
                ));
            }
        }
    }
    if !seen_header {
        return Err("patch parse error: missing 'syntax: v2' header".to_string());
    }
    Ok(rules)
}

/// Parse a target from the tokens after the verb. For rename, trailing
/// `-> new` tokens were already validated by the caller; here we consume
/// the leading kind + name tokens only.
fn parse_target(toks: &[&str], is_rename: bool, line: usize) -> Result<V2Target, String> {
    let kind = toks[0];
    let name_tok = toks.get(1).copied().unwrap_or("");
    let _ = is_rename;
    match kind {
        "gene" => parse_name_ordinal(name_tok)
            .map(|(name, ordinal)| V2Target::Gene { name, ordinal })
            .ok_or_else(|| format!("patch parse error at line {}: bad gene '{}'", line, name_tok)),
        "splice" => {
            if name_tok.is_empty() {
                return Err(format!("patch parse error at line {}: splice needs a root", line));
            }
            Ok(V2Target::Splice {
                root: name_tok.to_string(),
            })
        }
        "variant" => {
            let (root, name) = name_tok
                .split_once('.')
                .ok_or_else(|| {
                    format!(
                        "patch parse error at line {}: variant is ROOT.NAME, found '{}'",
                        line, name_tok
                    )
                })?;
            Ok(V2Target::Variant {
                root: root.to_string(),
                name: name.to_string(),
            })
        }
        "phenotype" => {
            if name_tok.is_empty() {
                return Err(format!("patch parse error at line {}: phenotype needs a name", line));
            }
            Ok(V2Target::Pheno {
                name: name_tok.to_string(),
            })
        }
        "method" => {
            let (pheno, gene) = name_tok.split_once('.').ok_or_else(|| {
                format!(
                    "patch parse error at line {}: method is PHENO.GENE, found '{}'",
                    line, name_tok
                )
            })?;
            Ok(V2Target::Method {
                pheno: pheno.to_string(),
                gene: gene.to_string(),
            })
        }
        "fate" => {
            if name_tok.is_empty() {
                return Err(format!("patch parse error at line {}: fate needs a name", line));
            }
            Ok(V2Target::Fate {
                name: name_tok.to_string(),
            })
        }
        "regulate" => {
            if is_rename {
                return Err(format!(
                    "patch parse error at line {}: regulate supports delete only",
                    line
                ));
            }
            if !name_tok.starts_with('#') {
                return Err(format!(
                    "patch parse error at line {}: regulate needs an explicit ordinal '#N'",
                    line
                ));
            }
            let n: usize = name_tok[1..].parse().map_err(|_| {
                format!("patch parse error at line {}: bad ordinal '{}'", line, name_tok)
            })?;
            if n == 0 {
                return Err(format!(
                    "patch parse error at line {}: ordinals are 1-indexed",
                    line
                ));
            }
            Ok(V2Target::Regulate { ordinal: n })
        }
        other => Err(format!(
            "patch parse error at line {}: unknown target kind '{}' (gene | splice | variant | phenotype | method | fate | regulate)",
            line, other
        )),
    }
}

/// `name` or `name#2`, ordinal must be >= 1.
fn parse_name_ordinal(tok: &str) -> Option<(String, Option<usize>)> {
    match tok.split_once('#') {
        Some((name, ord)) => {
            let n: usize = ord.parse().ok()?;
            if n == 0 {
                return None;
            }
            Some((name.to_string(), Some(n)))
        }
        None => Some((tok.to_string(), None)),
    }
}

/// Scan one body-content line for the matching close brace. Returns the
/// content that survives (text before the closing brace, if this line
/// closes) and the new depth.
fn scan_body_line(
    line: &str,
    mut depth: i64,
    mut in_str: bool,
    closed: &mut bool,
) -> (String, i64) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let n = chars.len();
    let mut end = n; // exclusive end of surviving content
    while i < n {
        let c = chars[i];
        if in_str {
            match c {
                '\\' => i += 1,
                '"' => in_str = false,
                _ => {}
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                i += 1;
            }
            '{' => {
                depth += 1;
                i += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    *closed = true;
                    break;
                }
                i += 1;
            }
            '#' => break, // comment tail, excluded from content
            _ => i += 1,
        }
    }
    let content: String = chars[..end].iter().collect();
    (content, depth)
}

/// Track the single-quote state across a whole line (for body gathering).
fn body_str_state(line: &str, mut in_str: bool) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            match c {
                '\\' => i += 1,
                '"' => in_str = false,
                _ => {}
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '#' {
            break;
        }
        i += 1;
    }
    in_str
}

// ------------------------------------------------------------ apply

/// Apply a v2 patch to source. Node-addressed, all-or-nothing, reprint via
/// the canonical formatter.
pub fn apply_rna_v2(
    src: &str,
    patch_src: &str,
    allow_comment_drop: bool,
) -> Result<Rna2Report, String> {
    let rules = parse_v2_patch(patch_src)?;
    let comment_lines = plain_comment_lines(src);
    if !comment_lines.is_empty() && !allow_comment_drop {
        return Err(comment_refusal_msg(&comment_lines));
    }

    // Parse, don't slice: a fresh AST from the CURRENT source.
    let mut prog = crate::parser::parse(src);

    let mut reports: Vec<Rna2RuleReport> = Vec::new();
    let mut any_miss = false;

    for rule in &rules {
        let target_text = rule.target.canonical();
        let outcome: Result<String, String> = match (&rule.verb, &rule.target) {
            (V2Verb::Rename(new), V2Target::Gene { name, ordinal }) => rename_named_decl(
                &mut prog,
                &|s: &Stmt| decl_gene_name(s).map(|n| n == name).unwrap_or(false),
                name.as_str(),
                *ordinal,
                |s| {
                    if let Stmt::Gene(g) | Stmt::Seq(g) = s {
                        Arc::make_mut(g).name = Some(new.clone());
                    }
                },
                &mut RewriteCfg {
                    ident: Some((name.as_str(), new.as_str())),
                    ..RewriteCfg::default()
                },
            )
            .map(|n| {
                format!(
                    "gene '{}' -> '{}' (decl + {} reference node(s))",
                    name, new, n
                )
            }),
            (V2Verb::Rename(new), V2Target::Splice { root }) => rename_named_decl(
                &mut prog,
                &|s: &Stmt| decl_splice_root(s).map(|r| r == root).unwrap_or(false),
                root.as_str(),
                None,
                |s| {
                    if let Stmt::Splice(sp) = s {
                        Arc::make_mut(sp).root = new.clone();
                    }
                },
                &mut RewriteCfg {
                    ident: Some((root.as_str(), new.as_str())),
                    ..RewriteCfg::default()
                },
            )
            .map(|n| {
                format!(
                    "splice '{}' -> '{}' (decl + {} reference node(s))",
                    root, new, n
                )
            }),
            (V2Verb::Rename(new), V2Target::Variant { root, name }) => {
                match find_decl(
                    &prog,
                    &|s: &Stmt| decl_splice_root(s).map(|r| r == root).unwrap_or(false),
                    root.as_str(),
                    None,
                ) {
                    Err(e) => Err(e),
                    Ok(path) => {
                        let stmt = stmt_at_path(&mut prog.stmts, &path);
                        if let Stmt::Splice(sp) = stmt {
                            let sd = Arc::make_mut(sp);
                            match sd.variants.iter_mut().find(|(n, _)| n == name) {
                                None => Err(format!("splice '{}' has no variant '{}'", root, name)),
                                Some((n, gd)) => {
                                    *n = new.clone();
                                    if gd.name.as_deref() == Some(name.as_str()) {
                                        let g = Arc::make_mut(gd);
                                        g.name = Some(new.clone());
                                    }
                                    Ok(format!("variant '{}.{}' -> '{}'", root, name, new))
                                }
                            }
                        } else {
                            Err("internal: splice path resolved to a non-splice".to_string())
                        }
                    }
                }
            }
            (V2Verb::Rename(new), V2Target::Pheno { name }) => rename_named_decl(
                &mut prog,
                &|s: &Stmt| decl_pheno_name(s).map(|n| n == name).unwrap_or(false),
                name.as_str(),
                None,
                |s| {
                    if let Stmt::Pheno(p) = s {
                        Arc::make_mut(p).name = new.clone();
                    }
                },
                &mut RewriteCfg {
                    ctor: Some((name.as_str(), new.as_str())),
                    ann: Some((name.as_str(), new.as_str())),
                    ..RewriteCfg::default()
                },
            )
            .map(|n| {
                format!(
                    "phenotype '{}' -> '{}' (decl + {} constructor/annotation node(s))",
                    name, new, n
                )
            }),
            (V2Verb::Rename(new), V2Target::Method { pheno, gene }) => {
                match find_decl(
                    &prog,
                    &|s: &Stmt| decl_pheno_name(s).map(|n| n == pheno).unwrap_or(false),
                    pheno.as_str(),
                    None,
                ) {
                    Err(e) => Err(e),
                    Ok(path) => {
                        let stmt = stmt_at_path(&mut prog.stmts, &path);
                        if let Stmt::Pheno(p) = stmt {
                            match p.methods.iter().filter(|m| m.name.as_deref() == Some(gene)).count() {
                                0 => Err(format!("phenotype '{}' has no method '{}'", pheno, gene)),
                                n if n > 1 => Err(format!(
                                    "ambiguous: {} methods named '{}' in phenotype '{}', rename the phenotype's methods one at a time",
                                    n, gene, pheno
                                )),
                                _ => {
                                    let pd = Arc::make_mut(p);
                                    for m in pd.methods.iter_mut() {
                                        if m.name.as_deref() == Some(gene) {
                                            Arc::make_mut(m).name = Some(new.clone());
                                        }
                                    }
                                    let refs = {
                                        let mut cfg = RewriteCfg {
                                            method: Some((gene.as_str(), new.as_str())),
                                            ..RewriteCfg::default()
                                        };
                                        rewrite_stmts(&mut prog.stmts, &mut cfg);
                                        cfg.count
                                    };
                                    Ok(format!(
                                        "method '{}.{}' -> '{}' (decl + {} call-site node(s))",
                                        pheno, gene, new, refs
                                    ))
                                }
                            }
                        } else {
                            Err("internal: phenotype path resolved to a non-phenotype".to_string())
                        }
                    }
                }
            }
            (V2Verb::Rename(new), V2Target::Fate { name }) => {
                rename_named_decl(
                    &mut prog,
                    &|s: &Stmt| decl_fate_name(s).map(|n| n == name).unwrap_or(false),
                    name.as_str(),
                    None,
                    |s| {
                        if let Stmt::Fate(f) = s {
                            Arc::make_mut(f).name = new.clone();
                        }
                    },
                    &mut RewriteCfg {
                        // a fate's call sites are plain Ident nodes, the
                        // parser never emits FateNew; the runtime fabricates
                        // it in call_value when the name is a fate registry
                        // hit. Rewriting Idents therefore rewrites the
                        // constructor calls too.
                        ident: Some((name.as_str(), new.as_str())),
                        ..RewriteCfg::default()
                    },
                )
                .map(|n| {
                    format!(
                        "fate '{}' -> '{}' (decl + {} reference node(s))",
                        name, new, n
                    )
                })
            }
            (V2Verb::Delete, V2Target::Gene { name, ordinal }) => {
                match find_decl(
                    &prog,
                    &|s: &Stmt| decl_gene_name(s).map(|n| n == name).unwrap_or(false),
                    name.as_str(),
                    *ordinal,
                ) {
                    Err(e) => Err(e),
                    Ok(path) => {
                        remove_at_path(&mut prog.stmts, &path);
                        Ok(format!(
                            "gene '{}' deleted (references now fail honestly)",
                            name
                        ))
                    }
                }
            }
            (V2Verb::Delete, V2Target::Splice { root }) => {
                match find_decl(
                    &prog,
                    &|s: &Stmt| decl_splice_root(s).map(|r| r == root).unwrap_or(false),
                    root.as_str(),
                    None,
                ) {
                    Err(e) => Err(e),
                    Ok(path) => {
                        remove_at_path(&mut prog.stmts, &path);
                        Ok(format!(
                            "splice '{}' deleted (variants detached honestly)",
                            root
                        ))
                    }
                }
            }
            (V2Verb::Delete, V2Target::Variant { root, name }) => {
                match find_decl(
                    &prog,
                    &|s: &Stmt| decl_splice_root(s).map(|r| r == root).unwrap_or(false),
                    root.as_str(),
                    None,
                ) {
                    Err(e) => Err(e),
                    Ok(path) => {
                        let stmt = stmt_at_path(&mut prog.stmts, &path);
                        if let Stmt::Splice(sp) = stmt {
                            let sd = Arc::make_mut(sp);
                            let before = sd.variants.len();
                            sd.variants.retain(|(n, _)| n != name);
                            if sd.variants.len() == before {
                                Err(format!("splice '{}' has no variant '{}'", root, name))
                            } else {
                                Ok(format!("variant '{}.{}' deleted", root, name))
                            }
                        } else {
                            Err("internal: splice path resolved to a non-splice".to_string())
                        }
                    }
                }
            }
            (V2Verb::Delete, V2Target::Pheno { name }) => {
                match find_decl(
                    &prog,
                    &|s: &Stmt| decl_pheno_name(s).map(|n| n == name).unwrap_or(false),
                    name.as_str(),
                    None,
                ) {
                    Err(e) => Err(e),
                    Ok(path) => {
                        remove_at_path(&mut prog.stmts, &path);
                        Ok(format!("phenotype '{}' deleted", name))
                    }
                }
            }
            (V2Verb::Delete, V2Target::Method { pheno, gene }) => {
                match find_decl(
                    &prog,
                    &|s: &Stmt| decl_pheno_name(s).map(|n| n == pheno).unwrap_or(false),
                    pheno.as_str(),
                    None,
                ) {
                    Err(e) => Err(e),
                    Ok(path) => {
                        let stmt = stmt_at_path(&mut prog.stmts, &path);
                        if let Stmt::Pheno(p) = stmt {
                            let pd = Arc::make_mut(p);
                            let before = pd.methods.len();
                            pd.methods.retain(|m| m.name.as_deref() != Some(gene));
                            if pd.methods.len() == before {
                                Err(format!("phenotype '{}' has no method '{}'", pheno, gene))
                            } else {
                                Ok(format!("method '{}.{}' deleted", pheno, gene))
                            }
                        } else {
                            Err("internal: phenotype path resolved to a non-phenotype".to_string())
                        }
                    }
                }
            }
            (V2Verb::Delete, V2Target::Fate { name }) => {
                match find_decl(
                    &prog,
                    &|s: &Stmt| decl_fate_name(s).map(|n| n == name).unwrap_or(false),
                    name.as_str(),
                    None,
                ) {
                    Err(e) => Err(e),
                    Ok(path) => {
                        remove_at_path(&mut prog.stmts, &path);
                        Ok(format!("fate '{}' deleted", name))
                    }
                }
            }
            (V2Verb::Delete, V2Target::Regulate { ordinal }) => {
                let pred_matches = |s: &Stmt| matches!(s, Stmt::Regulate(..));
                let mut paths = Vec::new();
                collect_decl_paths(&prog.stmts, &mut Vec::new(), &mut paths, &pred_matches);
                if *ordinal > paths.len() {
                    Err(format!(
                        "only {} regulate statement(s) declared, '#{}' does not exist",
                        paths.len(),
                        ordinal
                    ))
                } else {
                    let path = paths[*ordinal - 1].clone();
                    remove_at_path(&mut prog.stmts, &path);
                    Ok(format!("regulate statement #{} deleted", ordinal))
                }
            }
            (V2Verb::Body(text), V2Target::Gene { name, ordinal }) => {
                // parse the replacement FIRST, a parse error refuses the
                // whole apply (W067 design note §3)
                let frag = crate::parser::parse(text);
                let fatal: Vec<String> = frag
                    .notes
                    .iter()
                    .filter(|n| n.rung >= 4)
                    .map(|n| format!("line {}: {}", n.line, n.message))
                    .collect();
                if !fatal.is_empty() {
                    Err(format!(
                        "body replacement for '{}' does not parse cleanly: {}",
                        name,
                        fatal.join("; ")
                    ))
                } else {
                    match find_decl(
                        &prog,
                        &|s: &Stmt| decl_gene_name(s).map(|n| n == name).unwrap_or(false),
                        name.as_str(),
                        *ordinal,
                    ) {
                        Err(e) => Err(e),
                        Ok(path) => {
                            let stmt = stmt_at_path(&mut prog.stmts, &path);
                            if let Stmt::Gene(g) | Stmt::Seq(g) = stmt {
                                let gd = Arc::make_mut(g);
                                gd.body = frag.stmts.clone();
                                Ok(format!(
                                    "body of gene '{}' replaced ({} statement(s))",
                                    name,
                                    gd.body.len()
                                ))
                            } else {
                                Err("internal: gene path resolved to a non-gene".to_string())
                            }
                        }
                    }
                }
            }
            _ => Err("internal: verb/target combination unreachable".to_string()),
        };
        match outcome {
            Ok(detail) => reports.push(Rna2RuleReport {
                verb: verb_name(&rule.verb),
                target: target_text,
                target_found: true,
                applied: true,
                detail,
            }),
            Err(detail) => {
                any_miss = true;
                reports.push(Rna2RuleReport {
                    verb: verb_name(&rule.verb),
                    target: target_text,
                    target_found: false,
                    applied: false,
                    detail,
                });
            }
        }
    }

    if any_miss {
        // all-or-nothing: nothing is written, every fate is reported
        return Ok(Rna2Report {
            rules: reports,
            new_text: None,
        });
    }

    // Reprint, don't splice: the edited AST goes through the canonical
    // formatter, so apply output is fmt-stable by construction.
    let new_text = tools::format_program(&prog);
    Ok(Rna2Report {
        rules: reports,
        new_text: Some(new_text),
    })
}

fn verb_name(v: &V2Verb) -> String {
    match v {
        V2Verb::Rename(_) => "rename".to_string(),
        V2Verb::Delete => "delete".to_string(),
        V2Verb::Body(_) => "body".to_string(),
    }
}

// ------------------------------------------------------------ check (W68)

/// W68: result of validating a v2 patch against its target WITHOUT applying
/// anything. `check_rna_v2` runs the exact apply machinery in memory (the
/// apply path is pure: parse -> edit AST -> reprint, no file IO), so the
/// verdict is by construction what a real apply with the same flags would do.
#[derive(Debug, Clone)]
pub struct Rna2CheckReport {
    /// the patch itself did not parse (rules never ran)
    pub parse_error: Option<String>,
    /// per-rule fate under the in-memory apply. Empty when the patch never
    /// reached rule resolution: parse error or comment-preflight refusal,
    /// mirroring the apply path's order (parse -> preflight -> rules).
    pub rules: Vec<Rna2RuleReport>,
    /// plain `#` comment lines found in the TARGET source (the reprint would
    /// drop them)
    pub comment_lines: Vec<usize>,
    /// Some(refusal message) when the comment guard would refuse the apply
    pub comment_refusal: Option<String>,
    /// a real apply with the same flags would succeed end-to-end
    pub would_apply: bool,
    /// refusal reason whenever `would_apply` is false (patch parse error,
    /// comment refusal, or the all-or-nothing miss summary; per-rule detail
    /// lives in the rule rows)
    pub reason: Option<String>,
}

/// Validate a v2 patch against a target without applying anything. Same
/// gate order as `apply_rna_v2` (patch parse -> comment preflight -> rule
/// resolution), so check and apply can never disagree.
pub fn check_rna_v2(src: &str, patch_src: &str, allow_comment_drop: bool) -> Rna2CheckReport {
    let comment_lines = plain_comment_lines(src);
    let comment_refusal = if !comment_lines.is_empty() && !allow_comment_drop {
        Some(comment_refusal_msg(&comment_lines))
    } else {
        None
    };
    let mut rep = Rna2CheckReport {
        parse_error: None,
        rules: Vec::new(),
        comment_lines,
        comment_refusal,
        would_apply: false,
        reason: None,
    };
    if let Err(e) = parse_v2_patch(patch_src) {
        rep.parse_error = Some(e.clone());
        rep.reason = Some(e);
        return rep;
    }
    match apply_rna_v2(src, patch_src, allow_comment_drop) {
        Ok(r) => {
            let missed = r.missed();
            rep.rules = r.rules;
            if r.new_text.is_some() {
                rep.would_apply = true;
            } else {
                rep.reason = Some(format!("all-or-nothing: {} rule(s) missed", missed));
            }
        }
        Err(e) => {
            // only the comment preflight can refuse here (the patch already
            // parsed above); mirror the apply refusal verbatim
            rep.reason = Some(e);
        }
    }
    rep
}

// ------------------------------------------------------------ decl lookup

/// Paths of all declarations matching `kind` AND `name_pred`, in program
/// order (top level + TAD/Block bodies, the same scope class v1's
/// gene_span targeted; Frame and gene bodies are NOT decl scope).
fn collect_decl_paths(
    stmts: &[Stmt],
    path: &mut Vec<usize>,
    out: &mut Vec<Vec<usize>>,
    pred: &dyn Fn(&Stmt) -> bool,
) {
    for (i, s) in stmts.iter().enumerate() {
        path.push(i);
        if pred(s) {
            out.push(path.clone());
        }
        match s {
            Stmt::Tad(_, body) | Stmt::Block(body) => {
                collect_decl_paths(body, path, out, pred);
            }
            _ => {}
        }
        path.pop();
    }
}

/// Resolve one decl: exactly one match (or the requested ordinal). Bare
/// names with multiple matches refuse with an ordinal guide.
fn find_decl(
    prog: &Program,
    pred: &dyn Fn(&Stmt) -> bool,
    name: &str,
    ordinal: Option<usize>,
) -> Result<Vec<usize>, String> {
    let mut paths = Vec::new();
    collect_decl_paths(&prog.stmts, &mut Vec::new(), &mut paths, pred);
    match (ordinal, paths.len()) {
        (_, 0) => Err(format!("no declaration named '{}' found", name)),
        (Some(n), total) if n > total => Err(format!(
            "ordinal '{}' out of range: {} declaration(s) named '{}' (use #1..#{})",
            n,
            total,
            name,
            if total == 0 { 0 } else { total }
        )),
        (Some(n), _) => Ok(paths[n - 1].clone()),
        (None, 1) => Ok(paths[0].clone()),
        (None, total) => Err(format!(
            "ambiguous: {} declarations named '{}', address one by ordinal ({})",
            total,
            name,
            (1..=total)
                .map(|k| format!("{}#{}", name, k))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Find + mutate + rewrite references. Returns the reference-rewrite count.
fn rename_named_decl(
    prog: &mut Program,
    pred: &dyn Fn(&Stmt) -> bool,
    name: &str,
    ordinal: Option<usize>,
    mutate: impl Fn(&mut Stmt),
    cfg: &mut RewriteCfg,
) -> Result<usize, String> {
    let path = find_decl(prog, pred, name, ordinal)?;
    let stmt = stmt_at_path(&mut prog.stmts, &path);
    mutate(stmt);
    rewrite_stmts(&mut prog.stmts, cfg);
    Ok(cfg.count)
}

/// Remove the statement at `path` (its parent list is Tad/Block/program).
fn remove_at_path(stmts: &mut Vec<Stmt>, path: &[usize]) {
    if path.len() == 1 {
        stmts.remove(path[0]);
        return;
    }
    let head = path[0];
    let body = match &mut stmts[head] {
        Stmt::Tad(_, b) | Stmt::Block(b) => b,
        _ => return,
    };
    remove_at_path(body, &path[1..]);
}

/// Mutable reference to the statement at `path` (descends Tad/Block).
fn stmt_at_path<'a>(stmts: &'a mut [Stmt], path: &[usize]) -> &'a mut Stmt {
    let mut cur = &mut stmts[path[0]];
    for &ix in &path[1..] {
        let body = match cur {
            Stmt::Tad(_, b) | Stmt::Block(b) => b,
            _ => unreachable!("decl paths only descend Tad/Block"),
        };
        cur = &mut body[ix];
    }
    cur
}

fn decl_gene_name(s: &Stmt) -> Option<&str> {
    match s {
        Stmt::Gene(g) | Stmt::Seq(g) => g.name.as_deref(),
        _ => None,
    }
}
fn decl_splice_root(s: &Stmt) -> Option<&str> {
    match s {
        Stmt::Splice(sp) => Some(&sp.root),
        _ => None,
    }
}
fn decl_pheno_name(s: &Stmt) -> Option<&str> {
    match s {
        Stmt::Pheno(p) => Some(&p.name),
        _ => None,
    }
}
fn decl_fate_name(s: &Stmt) -> Option<&str> {
    match s {
        Stmt::Fate(f) => Some(&f.name),
        _ => None,
    }
}

// ------------------------------------------------------------ rewriter

/// Which reference nodes get rewritten by the current rename, plus a
/// mutation counter for the fate report.
#[derive(Default)]
pub struct RewriteCfg<'a> {
    /// Ident(from) -> Ident(to), gene/splice-root references
    ident: Option<(&'a str, &'a str)>,
    /// New(from, ...) -> New(to, ...), phenotype constructor calls
    ctor: Option<(&'a str, &'a str)>,

    /// Method(_, from, ...), method name strings
    method: Option<(&'a str, &'a str)>,
    /// TypeAnn::Named(from) -> Named(to), phenotype annotations
    ann: Option<(&'a str, &'a str)>,
    /// external counter (None inside nested walks that must not double-count)
    counter: Option<&'a mut usize>,
    count: usize,
}

impl<'a> RewriteCfg<'a> {
    fn bump(&mut self) {
        self.count += 1;
        if let Some(c) = self.counter.as_mut() {
            **c += 1;
        }
    }
}

fn rewrite_ann(ann: &mut TypeAnn, cfg: &mut RewriteCfg) {
    match ann {
        TypeAnn::Named(s) => {
            if let Some((from, to)) = cfg.ann {
                if s == from {
                    *s = to.to_string();
                    cfg.bump();
                }
            }
        }
        TypeAnn::Union(alts) => {
            for a in alts {
                rewrite_ann(a, cfg);
            }
        }
        TypeAnn::Optional(inner) => rewrite_ann(inner, cfg),
        // W01-s2: an alias descends into its target (its NAME is the
        // alias identity, never rewritten)
        TypeAnn::Alias { target, .. } => rewrite_ann(target, cfg),
        // TYPED-MODE: generic annotations rewrite their head name and
        // descend into the type arguments (same rename law as Named)
        TypeAnn::Generic(head, args) => {
            if let Some((from, to)) = cfg.ann {
                if head == from {
                    *head = to.to_string();
                    cfg.bump();
                }
            }
            for a in args {
                rewrite_ann(a, cfg);
            }
        }
    }
}

fn rewrite_anns(anns: &mut [Option<TypeAnn>], cfg: &mut RewriteCfg) {
    for a in anns.iter_mut().flatten() {
        rewrite_ann(a, cfg);
    }
}

fn rewrite_pat(p: &mut MatchPat, cfg: &mut RewriteCfg) {
    match p {
        MatchPat::Lit(e) => rewrite_expr(e, cfg),
        MatchPat::Multi(es) => {
            for e in es {
                rewrite_expr(e, cfg);
            }
        }
        MatchPat::Bind(_) | MatchPat::Wild => {}
        MatchPat::Variant(_, sub) => {
            if let Some(sub) = sub {
                rewrite_pat(sub, cfg);
            }
        }
        MatchPat::ListPat { elems, .. } => {
            for e in elems {
                rewrite_pat(e, cfg);
            }
        }
        MatchPat::MapPat { keys } => {
            for (_, sub) in keys.iter_mut() {
                if let Some(sub) = sub {
                    rewrite_pat(sub, cfg);
                }
            }
        }
        MatchPat::Or(alts) => {
            for a in alts {
                rewrite_pat(a, cfg);
            }
        }
        MatchPat::Guard(pat, cond) => {
            rewrite_pat(pat, cfg);
            rewrite_expr(cond, cfg);
        }
    }
}

fn rewrite_expr(e: &mut Expr, cfg: &mut RewriteCfg) {
    match e {
        Expr::Ident(s) => {
            if let Some((from, to)) = cfg.ident {
                if s == from {
                    *s = to.to_string();
                    cfg.bump();
                }
            }
        }
        // W029: bytes literals are leaves, nothing to rewrite inside
        Expr::Bytes(_) => {}
        Expr::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(sub) = p {
                    rewrite_expr(sub, cfg);
                }
            }
        }
        Expr::List(items) => {
            for it in items {
                rewrite_expr(it, cfg);
            }
        }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                rewrite_expr(k, cfg);
                rewrite_expr(v, cfg);
            }
        }
        Expr::Unary(_, a) => rewrite_expr(a, cfg),
        Expr::Binary(_, a, b, _) => {
            rewrite_expr(a, cfg);
            rewrite_expr(b, cfg);
        }
        Expr::Call(f, args, _) => {
            rewrite_expr(f, cfg);
            for a in args {
                rewrite_expr(a, cfg);
            }
        }
        Expr::Index(a, b, _) => {
            rewrite_expr(a, cfg);
            rewrite_expr(b, cfg);
        }
        Expr::Member(a, _) | Expr::MemberSafe(a, _) => rewrite_expr(a, cfg),
        Expr::Method(a, name, args, _) | Expr::MethodSafe(a, name, args, _) => {
            rewrite_expr(a, cfg);
            if let Some((from, to)) = cfg.method {
                if name == from {
                    *name = to.to_string();
                    cfg.bump();
                }
            }
            for a in args {
                rewrite_expr(a, cfg);
            }
        }
        Expr::Lambda(g) => {
            let gd = Arc::make_mut(g);
            for (_, d) in gd.params.iter_mut() {
                if let Some(d) = d {
                    rewrite_expr(d, cfg);
                }
            }
            if let Some((c, b)) = &mut gd.guard {
                rewrite_expr(c, cfg);
                rewrite_stmts(b, cfg);
            }
            rewrite_stmts(&mut gd.body, cfg);
        }
        Expr::Collect {
            iter, filter, body, ..
        } => {
            rewrite_expr(iter, cfg);
            if let Some(f) = filter {
                rewrite_expr(f, cfg);
            }
            rewrite_expr(body, cfg);
        }
        // FateNew is never parsed, the runtime fabricates it when a call's
        // name resolves to a fate (interp call_value). Nothing to rewrite.
        Expr::FateNew(_) => {}
        Expr::New(name, args) => {
            if let Some((from, to)) = cfg.ctor {
                if name == from {
                    *name = to.to_string();
                    cfg.bump();
                }
            }
            for a in args {
                rewrite_expr(a, cfg);
            }
        }
        Expr::Ternary(a, b, c) => {
            rewrite_expr(a, cfg);
            rewrite_expr(b, cfg);
            rewrite_expr(c, cfg);
        }
        Expr::Propagate(a, _) => rewrite_expr(a, cfg),
        Expr::Null | Expr::Bool(_) | Expr::Int(_) | Expr::Float(_) | Expr::Str(_) => {}
    }
}

fn rewrite_stmt(s: &mut Stmt, cfg: &mut RewriteCfg) {
    match s {
        // W01-s2: alias declarations rewrite their target annotation
        Stmt::TypeAlias(_, target, _) => rewrite_ann(target, cfg),
        Stmt::Let(_, e)
        | Stmt::LetConst(_, e)
        | Stmt::Assign(_, _, e)
        | Stmt::Return(Some(e))
        | Stmt::ExprStmt(e) => rewrite_expr(e, cfg),
        Stmt::LetAnn(_, ann, e) => {
            rewrite_ann(ann, cfg);
            rewrite_expr(e, cfg);
        }
        Stmt::IndexAssign(a, b, _, c) => {
            rewrite_expr(a, cfg);
            rewrite_expr(b, cfg);
            rewrite_expr(c, cfg);
        }
        Stmt::MemberAssign(a, _, _, b) => {
            rewrite_expr(a, cfg);
            rewrite_expr(b, cfg);
        }
        Stmt::LetPat(_, e) => rewrite_expr(e, cfg),
        Stmt::ForPat(_, e, body) => {
            rewrite_expr(e, cfg);
            rewrite_stmts(body, cfg);
        }
        Stmt::MultiAssign(targets, sources, _) => {
            for t in targets {
                rewrite_expr(t, cfg);
            }
            for src in sources {
                rewrite_expr(src, cfg);
            }
        }
        Stmt::If(arms, else_body) => {
            for (cond, body) in arms {
                rewrite_expr(cond, cfg);
                rewrite_stmts(body, cfg);
            }
            if let Some(b) = else_body {
                rewrite_stmts(b, cfg);
            }
        }
        Stmt::While(cond, body) => {
            rewrite_expr(cond, cfg);
            rewrite_stmts(body, cfg);
        }
        Stmt::Loop(body)
        | Stmt::Scope(body)
        | Stmt::Frame { body, .. }
        | Stmt::Tad(_, body)
        // W025 stage 2: fix rewrites reach inside nested sub-module tables
        | Stmt::Module(_, body)
        | Stmt::Block(body) => rewrite_stmts(body, cfg),
        Stmt::For(_, it, body) => {
            rewrite_expr(it, cfg);
            rewrite_stmts(body, cfg);
        }
        Stmt::Match(subject, arms, _) => {
            rewrite_expr(subject, cfg);
            for (pat, body) in arms {
                rewrite_pat(pat, cfg);
                rewrite_stmts(body, cfg);
            }
        }
        Stmt::Raise(_, e, _) => rewrite_expr(e, cfg),
        Stmt::Stress { body, rescue, .. } => {
            rewrite_stmts(body, cfg);
            if let Some((_, rb)) = rescue {
                rewrite_stmts(rb, cfg);
            }
        }
        Stmt::Gene(g) | Stmt::Seq(g) => {
            let gd = Arc::make_mut(g);
            rewrite_anns(&mut gd.param_anns, cfg);
            if let Some(ra) = &mut gd.ret_ann {
                rewrite_ann(ra, cfg);
            }
            for (_, d) in gd.params.iter_mut() {
                if let Some(d) = d {
                    rewrite_expr(d, cfg);
                }
            }
            if let Some((c, b)) = &mut gd.guard {
                rewrite_expr(c, cfg);
                rewrite_stmts(b, cfg);
            }
            rewrite_stmts(&mut gd.body, cfg);
        }
        Stmt::Splice(sp) => {
            let sd = Arc::make_mut(sp);
            for (_, gd) in sd.variants.iter_mut() {
                let g = Arc::make_mut(gd);
                rewrite_stmts(&mut g.body, cfg);
            }
        }
        Stmt::Pheno(p) => {
            let pd = Arc::make_mut(p);
            for (_, fe) in pd.fields.iter_mut() {
                rewrite_expr(fe, cfg);
            }
            for m in pd.methods.iter_mut() {
                let g = Arc::make_mut(m);
                rewrite_stmts(&mut g.body, cfg);
            }
        }
        // W04: trait default bodies are ordinary gene bodies, rewrite them;
        // required methods have no body.
        Stmt::Trait(t) => {
            let td = Arc::make_mut(t);
            for m in td.methods.iter_mut() {
                if let Some(g) = &mut m.default {
                    let gd = Arc::make_mut(g);
                    rewrite_stmts(&mut gd.body, cfg);
                }
            }
        }
        Stmt::Yield(Some(e)) => rewrite_expr(e, cfg),
        Stmt::Edit(..)
        | Stmt::Use(..)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Return(None)
        | Stmt::Yield(None)
        | Stmt::Silence(..)
        | Stmt::Operon(..)
        | Stmt::Enhance(..)
        | Stmt::Ires(..)
        | Stmt::Fate(_)
        | Stmt::Regulate(..)
        | Stmt::Ligand(_)
        | Stmt::Autoinducer(_)
        | Stmt::Toggle(..)
        | Stmt::Decoy(..)
        | Stmt::Repressilator(..)
        | Stmt::AnchorExport(_)
        | Stmt::AnchorImport(_) => {}
    }
}

fn rewrite_stmts(stmts: &mut [Stmt], cfg: &mut RewriteCfg) {
    for s in stmts.iter_mut() {
        rewrite_stmt(s, cfg);
    }
}
