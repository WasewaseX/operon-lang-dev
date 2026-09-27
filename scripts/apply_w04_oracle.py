#!/usr/bin/env python3
"""W04 traits oracle mirror: trait stmt + implements + default dispatch.
Precise anchored replacements on bootstrap/oracle.py; run from repo root."""
import sys

P = "bootstrap/oracle.py"
src = open(P, encoding="utf-8").read()

def rep(old, new):
    global src
    if new in src:
        return  # already applied
    if old not in src:
        print("ANCHOR MISS:", old[:70].replace("\n", "\\n"))
        sys.exit(1)
    if src.count(old) != 1:
        print("ANCHOR NOT UNIQUE:", old[:70].replace("\n", "\\n"), "count=", src.count(old))
        sys.exit(1)
    src = src.replace(old, new, 1)

# 1. Trait/TraitM classes after Pheno
rep('''class Pheno:
    __slots__ = ("name", "parent", "fields", "methods")
    def __init__(self, name, parent, fields, methods):
        self.name, self.parent, self.fields, self.methods = name, parent, fields, methods''',
    '''class Pheno:
    __slots__ = ("name", "parent", "fields", "methods", "implements")
    def __init__(self, name, parent, fields, methods, implements=None):
        self.name, self.parent, self.fields, self.methods = name, parent, fields, methods
        # W04: implemented traits, in declaration order (SPEC 8b)
        self.implements = implements or []

class TraitM:
    """W04: one trait method contract. required=True means the implementing
    phenotype must provide it; default is the full GeneDef body otherwise."""
    __slots__ = ("name", "required", "default")
    def __init__(self, name, required, default):
        self.name, self.required, self.default = name, required, default

class Trait:
    __slots__ = ("name", "methods")
    def __init__(self, name, methods):
        self.name, self.methods = name, methods''')

# 2. registry in __init__
rep("        self.phenos = {}",
    "        self.phenos = {}\n        # W04: trait registry (name -> Trait); last declaration wins\n        self.traits = {}")

# 3. phenotype header: implements clause
rep('''        if word == "phenotype":
            self.next()
            name = self.ident()
            parent = None
            if self.at_ident("from"):
                self.next()
                parent = self.ident()
            fields, methods = [], []''',
    '''        if word == "phenotype":
            self.next()
            name = self.ident()
            parent = None
            if self.at_ident("from"):
                self.next()
                parent = self.ident()
            # W04: optional `implements A, B, C` (contextual word)
            implements = []
            if self.at_ident("implements"):
                self.next()
                while True:
                    t = self.peek()
                    if t[0] == "IDENT":
                        self.next()
                        implements.append(t[1])
                    if self.peek() == ("SYM", ",", self.peek()[2]):
                        self.next()
                        continue
                    break
            fields, methods = [], []''')

# 4. phenotype return: pass implements
rep('            return ("pheno", Pheno(name, parent, fields, methods))',
    '            return ("pheno", Pheno(name, parent, fields, methods, implements))')

# 5. trait statement parse (before the bare-name definition lookahead)
rep('''        if word == "gene":
            self.next()
            return self.gene_def([])''',
    '''        if word == "trait":
            # W04 (SPEC 8b): `trait Name { gene m(); gene n() { ... } }` —
            # mirror of the Rust parser. A method with a body is a DEFAULT
            # (parsed by the normal gene parser); one without is REQUIRED.
            self.next()
            tname = self.ident()
            methods = []
            if self.peek() == ("SYM", "{", self.peek()[2]):
                self.next()
                while True:
                    self.eat_nl()
                    t = self.peek()
                    if t[0] == "SYM" and t[1] == "}":
                        self.next(); break
                    if t[0] == "EOF":
                        self.note(t[2], 4, "trait body auto-closed")
                        break
                    if t[0] == "IDENT" and (t[1] == "gene" or SYNONYMS.get(t[1]) == "gene"):
                        if self._trait_method_has_body():
                            self.next()  # consume 'gene' — gene_def parses from the name
                            g = self.gene_def([])
                            if g[0] == "gene":
                                methods.append(TraitM(g[1].name or "?", False, g[1]))
                            else:
                                self.note(t[2], 4, "trait method default body lost; treated as required")
                                methods.append(TraitM("?", True, None))
                        else:
                            self.next()
                            mname = self.ident()
                            self._trait_params()
                            self.end_stmt()
                            methods.append(TraitM(mname, True, None))
                        continue
                    self.note(t[2], 4, "unexpected token in trait body; skipped")
                    self.next()
            else:
                self.note(self.peek()[2], 4, "trait without body declares no methods")
            return ("trait", Trait(tname, methods))
        if word == "gene":
            self.next()
            return self.gene_def([])''')

# 6. helper methods (peek-ahead + params) — insert before `def stmt(self):`
rep('''    def stmt(self):
        t = self.peek()''',
    '''    def _trait_method_has_body(self):
        # W04: from the current 'gene' token, scan ahead past the matching
        # parameter parens; a '{' after the close paren means a default body.
        i = self.pos + 1
        n = len(self.toks)
        depth = 0
        seen_paren = False
        while i < n:
            t = self.toks[i]
            if t[0] == "SYM" and t[1] == "(":
                depth += 1
                seen_paren = True
            elif t[0] == "SYM" and t[1] == ")":
                depth -= 1
                if seen_paren and depth <= 0:
                    nx = self.toks[i + 1] if i + 1 < n else ("EOF", None, 0)
                    return nx[0] == "SYM" and nx[1] == "{"
            elif not seen_paren and t[0] == "SYM" and t[1] == "{":
                return True
            i += 1
        return False

    def _trait_params(self):
        # W04: consume `(a, b, ...)` — plain names (a signature, not a body).
        if not (self.peek()[0] == "SYM" and self.peek()[1] == "("):
            return []
        self.next()
        names = []
        while True:
            t = self.peek()
            if t[0] == "SYM" and t[1] == ")":
                self.next()
                break
            if t[0] == "IDENT":
                self.next()
                names.append(t[1])
                if t[1] == "Eq" or (self.peek()[0] == "SYM" and self.peek()[1] == "="):
                    self.next()
                    self.expr()
            elif t[0] == "EOF":
                break
            else:
                self.next()
        return names

    def stmt(self):
        t = self.peek()''')

# 7. exec: trait registration (next to pheno)
rep('''        elif k == "pheno":
            p = s[1]
            self.phenos[p.name] = p''',
    '''        elif k == "pheno":
            p = s[1]
            self.phenos[p.name] = p
        elif k == "trait":
            # W04: registry, last declaration wins (redefinition noted)
            t = s[1]
            if t.name in self.traits:
                self.note(4, f"redefining trait '{t.name}'")
            self.traits[t.name] = t''')

# 8. construct_obj: contract notes
rep('''        obj = ObjInst(p, fields)
        for dd in reversed(chain):
            for g in dd.methods:
                if g.name == "init":
                    self.call_method_gene(g, obj, args)
                    break
            else:
                continue
            break
        return obj''',
    '''        obj = ObjInst(p, fields)
        # W04: trait contract check — required methods must be provided by
        # the lineage; a break is a NOTE (Total Grammar), never fatal.
        for tname in p.implements:
            t = self.traits.get(tname)
            if t is None:
                self.note(4, f"trait '{tname}' not declared; contract on '{p.name}' ignored")
                continue
            for req in t.methods:
                if not req.required:
                    continue
                provided = any(g.name == req.name for dd in chain for g in dd.methods)
                if not provided:
                    self.note(4, f"phenotype '{p.name}' implements '{tname}' but does not provide '{req.name}()'")
        for dd in reversed(chain):
            for g in dd.methods:
                if g.name == "init":
                    self.call_method_gene(g, obj, args)
                    break
            else:
                continue
            break
        return obj''')

# 9. method dispatch: trait default fallback before field-callable fallback
rep('''            for dd in chain:
                for g in dd.methods:
                    if g.name == name:
                        return self.call_method_gene(g, recv, args)
            if name in recv.fields:
                return self.call_value(env, recv.fields[name], args)''',
    '''            for dd in chain:
                for g in dd.methods:
                    if g.name == name:
                        return self.call_method_gene(g, recv, args)
            # W04: trait default methods — implemented traits in declaration
            # order; virtual (self.x() inside dispatches through the lineage
            # first) because the fallback resolution is the same walk.
            for tname in recv.defn.implements:
                t = self.traits.get(tname)
                if t is None:
                    continue
                for m in t.methods:
                    if m.name == name and m.default is not None:
                        return self.call_method_gene(m.default, recv, args)
            if name in recv.fields:
                return self.call_value(env, recv.fields[name], args)''')

open(P, "w", encoding="utf-8").write(src)
print("W04 oracle mirror applied")
