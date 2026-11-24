//! The field calculator (the maintainer's card of 2026-11-24, "Full set
//! plus a formula library"): your own formulas of the open result's
//! fields, which then join its fields as the derived quantities do
//! ([`crate::derive`]). A formula is read by recursive descent into a
//! tree, its names found among the result's fields and its types
//! (scalar or vector) checked before anything is computed. The
//! derivatives (`grad`, `curl`, `div`) are taken first, each as a whole
//! field, by the least-squares gradients of the vortex quantities
//! ([`crate::gradient`]); the rest is evaluated cell by cell (node by
//! node, face by face) on all threads.
//!
//! The language:
//! - numbers (`1`, `2.5`, `1e-3`) and `pi`; the coordinates `x`, `y`,
//!   `z` and the position vector `pos` of each cell centroid, node or
//!   boundary face centre;
//! - field names, bare (`p`, `U`) or quoted (`"p rgh"`, `'T.mean'`); a
//!   field wins over a built-in name of the same spelling, and a quoted
//!   name is always a field;
//! - `+ - * / ^` (`^` binds tightest and to the right), unary minus, and
//!   the comparisons `< <= > >=`, which give 1 or 0;
//! - the components `U.x`, `U.y`, `U.z` and vectors `[a, b, c]`;
//! - the functions of [`FUNCTIONS`].
//!
//! A vector adds to or subtracts from a vector, and a scalar times a
//! vector or a vector by a scalar is a vector; a vector times a vector
//! is refused, so that `dot` or `cross` says which is meant.
//!
//! Where the result sits: wherever every field it reads is (cells,
//! nodes, boundary faces); a formula with a derivative only in the
//! cells. When no one place has them all, the cells, with node fields
//! as the mean of each cell's nodes. A value that is not finite (the
//! square root of a negative number, a division by zero) is no value.

use std::borrow::Cow;

use murmel_mesh::Mesh;
use murmel_mesh::field::{Field, Location};
use serde::{Deserialize, Serialize};

use crate::derive::{Quantity, Refusal};
use crate::field::FieldSet;

/// One formula of your own: the field it makes, the formula, and the
/// unit it is shown with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Formula {
    pub name: String,
    pub formula: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// The longest formula, in characters.
pub const MAX_FORMULA: usize = 2000;
/// The longest field name a formula makes, and the longest unit.
pub const MAX_NAME: usize = 64;
/// How deeply brackets and functions may nest.
const MAX_DEPTH: usize = 64;

/// One function of the calculator.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Function {
    pub name: &'static str,
    /// Its arguments: `s` a scalar, `v` a vector, `t` either (the same
    /// for each `t`).
    pub args: &'static str,
    /// What it gives: `s`, `v`, or `t`.
    pub gives: &'static str,
    pub summary: &'static str,
}

const fn func(
    name: &'static str,
    args: &'static str,
    gives: &'static str,
    summary: &'static str,
) -> Function {
    Function {
        name,
        args,
        gives,
        summary,
    }
}

/// Every function of the calculator, in the order the window lists them.
pub const FUNCTIONS: &[Function] = &[
    func("sqrt", "s", "s", "square root"),
    func("abs", "s", "s", "absolute value"),
    func("exp", "s", "s", "e to the power"),
    func("log", "s", "s", "natural logarithm"),
    func("log10", "s", "s", "logarithm to base 10"),
    func("sin", "s", "s", "sine (radians)"),
    func("cos", "s", "s", "cosine (radians)"),
    func("tan", "s", "s", "tangent (radians)"),
    func("asin", "s", "s", "arcsine (radians)"),
    func("acos", "s", "s", "arccosine (radians)"),
    func("atan", "s", "s", "arctangent (radians)"),
    func("atan2", "s, s", "s", "atan2(y, x): the angle of (x, y)"),
    func("sinh", "s", "s", "hyperbolic sine"),
    func("cosh", "s", "s", "hyperbolic cosine"),
    func("tanh", "s", "s", "hyperbolic tangent"),
    func("sign", "s", "s", "-1, 0 or 1"),
    func("floor", "s", "s", "largest whole number not above"),
    func("ceil", "s", "s", "smallest whole number not below"),
    func("min", "s, s", "s", "the smaller"),
    func("max", "s, s", "s", "the larger"),
    func(
        "if",
        "s, t, t",
        "t",
        "if(c, a, b): a where c is not 0, else b",
    ),
    func(
        "mag",
        "t",
        "s",
        "magnitude of a vector (absolute value of a scalar)",
    ),
    func("dot", "v, v", "s", "dot product"),
    func("cross", "v, v", "v", "cross product"),
    func("grad", "s", "v", "gradient, in the cells"),
    func("curl", "v", "v", "curl, in the cells"),
    func("div", "v", "s", "divergence, in the cells"),
];

/// Whether `name` can be the name of a formula's field: a letter or `_`
/// first, then letters, digits and `_`, at most [`MAX_NAME`] long, and
/// not the field of a derived quantity.
pub fn check_name(name: &str) -> Result<(), Refusal> {
    let mut chars = name.chars();
    let first = chars.next();
    if !first.is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        || name.len() > MAX_NAME
    {
        return Err(Refusal::BadName(format!(
            "{name:?} cannot name a field: a letter or _ first, then letters, digits and _, at most {MAX_NAME} long"
        )));
    }
    if Quantity::of_field(name).is_some() {
        return Err(Refusal::BadName(format!(
            "{name} is the field of a derived quantity (field.derive makes it); choose another name"
        )));
    }
    Ok(())
}

/// The checks of a formula that need no result: its name, its unit, its
/// length and its grammar (a formula kept in your library is checked
/// so).
pub fn check(f: &Formula) -> Result<(), Refusal> {
    check_name(&f.name)?;
    if let Some(u) = &f.unit
        && (u.chars().count() > MAX_NAME || u.chars().any(char::is_control))
    {
        return Err(Refusal::BadName(format!(
            "the unit must be at most {MAX_NAME} characters, without control characters"
        )));
    }
    parse(&f.formula).map(|_| ())
}

/// The fields of `set` that `formula` reads, each once, in the order it
/// names them; empty when it cannot be read.
pub fn reads(formula: &str, set: &FieldSet) -> Vec<String> {
    parse(formula)
        .and_then(|ast| plan(&ast, set))
        .map(|p| p.fields)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------
// Reading the formula.

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    Quoted(String),
    Sym(&'static str),
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    /// Its column, from 1.
    at: usize,
}

const SYMBOLS: [&str; 15] = [
    "<=", ">=", "+", "-", "*", "/", "^", "(", ")", ",", "[", "]", ".", "<", ">",
];

fn syntax(at: usize, why: impl std::fmt::Display) -> Refusal {
    Refusal::Syntax(format!("{why} (column {at})"))
}

fn lex(text: &str) -> Result<Vec<Token>, Refusal> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() > MAX_FORMULA {
        return Err(Refusal::Syntax(format!(
            "the formula is {} characters long; at most {MAX_FORMULA}",
            chars.len()
        )));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let at = i + 1;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let digit = |k: usize| chars.get(k).is_some_and(char::is_ascii_digit);
        if c.is_ascii_digit() || (c == '.' && digit(i + 1)) {
            let start = i;
            while digit(i) {
                i += 1;
            }
            if chars.get(i) == Some(&'.') {
                i += 1;
                while digit(i) {
                    i += 1;
                }
            }
            if matches!(chars.get(i), Some('e' | 'E')) {
                let sign = usize::from(matches!(chars.get(i + 1), Some('+' | '-')));
                if digit(i + 1 + sign) {
                    i += 1 + sign;
                    while digit(i) {
                        i += 1;
                    }
                }
            }
            let s: String = chars[start..i].iter().collect();
            let v: f64 = s
                .parse()
                .map_err(|_| syntax(at, format!("{s} is not a number")))?;
            if !v.is_finite() {
                return Err(syntax(at, format!("{s} is too large")));
            }
            out.push(Token {
                tok: Tok::Num(v),
                at,
            });
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while chars
                .get(i)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
            {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Name(chars[start..i].iter().collect()),
                at,
            });
            continue;
        }
        if c == '"' || c == '\'' {
            let Some(len) = chars[i + 1..].iter().position(|&d| d == c) else {
                return Err(syntax(at, format!("the name in {c} has no closing {c}")));
            };
            let name: String = chars[i + 1..i + 1 + len].iter().collect();
            if name.is_empty() {
                return Err(syntax(at, "an empty name"));
            }
            out.push(Token {
                tok: Tok::Quoted(name),
                at,
            });
            i += len + 2;
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let Some(sym) = SYMBOLS.iter().find(|s| two.starts_with(**s)) else {
            return Err(syntax(at, format!("{c} is not part of a formula")));
        };
        out.push(Token {
            tok: Tok::Sym(sym),
            at,
        });
        i += sym.len();
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Op {
    fn symbol(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Pow => "^",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
        }
    }
}

/// The formula as read, before its names are found.
#[derive(Clone, Debug)]
enum Ast {
    Num(f64),
    Name {
        name: String,
        quoted: bool,
        at: usize,
    },
    Neg(Box<Ast>),
    Bin(Op, Box<Ast>, Box<Ast>, usize),
    Comp(Box<Ast>, usize, usize),
    Vector(Box<[Ast; 3]>),
    Call(String, Vec<Ast>, usize),
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
    /// The column after the last character, for "ends early".
    end: usize,
}

fn parse(text: &str) -> Result<Ast, Refusal> {
    let toks = lex(text)?;
    if toks.is_empty() {
        return Err(Refusal::Syntax("the formula is empty".into()));
    }
    let mut p = Parser {
        toks,
        pos: 0,
        depth: 0,
        end: text.chars().count() + 1,
    };
    let ast = p.compare()?;
    if let Some(t) = p.toks.get(p.pos) {
        return Err(syntax(
            t.at,
            format!("{} is not expected here", show(&t.tok)),
        ));
    }
    Ok(ast)
}

fn show(t: &Tok) -> String {
    match t {
        Tok::Num(v) => v.to_string(),
        Tok::Name(n) => n.clone(),
        Tok::Quoted(n) => format!("\"{n}\""),
        Tok::Sym(s) => (*s).to_string(),
    }
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn at(&self) -> usize {
        self.toks.get(self.pos).map_or(self.end, |t| t.at)
    }

    fn eat(&mut self, sym: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(s)) if *s == sym) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, sym: &str, what: &str) -> Result<(), Refusal> {
        if self.eat(sym) {
            return Ok(());
        }
        let found = match self.peek() {
            Some(t) => format!("found {}", show(t)),
            None => "the formula ends".into(),
        };
        Err(syntax(self.at(), format!("{what} needs {sym}; {found}")))
    }

    fn deeper(&mut self) -> Result<(), Refusal> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(syntax(
                self.at(),
                format!("brackets and functions nest more than {MAX_DEPTH} deep"),
            ));
        }
        Ok(())
    }

    fn compare(&mut self) -> Result<Ast, Refusal> {
        let a = self.sum()?;
        let at = self.at();
        let op = if self.eat("<=") {
            Op::Le
        } else if self.eat(">=") {
            Op::Ge
        } else if self.eat("<") {
            Op::Lt
        } else if self.eat(">") {
            Op::Gt
        } else {
            return Ok(a);
        };
        let b = self.sum()?;
        if matches!(self.peek(), Some(Tok::Sym("<" | ">" | "<=" | ">="))) {
            return Err(syntax(
                self.at(),
                "comparisons do not chain; join them with min, max or *",
            ));
        }
        Ok(Ast::Bin(op, Box::new(a), Box::new(b), at))
    }

    fn sum(&mut self) -> Result<Ast, Refusal> {
        let mut a = self.term()?;
        loop {
            let at = self.at();
            let op = if self.eat("+") {
                Op::Add
            } else if self.eat("-") {
                Op::Sub
            } else {
                return Ok(a);
            };
            let b = self.term()?;
            a = Ast::Bin(op, Box::new(a), Box::new(b), at);
        }
    }

    fn term(&mut self) -> Result<Ast, Refusal> {
        let mut a = self.unary()?;
        loop {
            let at = self.at();
            let op = if self.eat("*") {
                Op::Mul
            } else if self.eat("/") {
                Op::Div
            } else {
                return Ok(a);
            };
            let b = self.unary()?;
            a = Ast::Bin(op, Box::new(a), Box::new(b), at);
        }
    }

    fn unary(&mut self) -> Result<Ast, Refusal> {
        if self.eat("-") {
            self.deeper()?;
            let a = self.unary()?;
            self.depth -= 1;
            return Ok(Ast::Neg(Box::new(a)));
        }
        if self.eat("+") {
            self.deeper()?;
            let a = self.unary()?;
            self.depth -= 1;
            return Ok(a);
        }
        self.power()
    }

    fn power(&mut self) -> Result<Ast, Refusal> {
        let a = self.postfix()?;
        let at = self.at();
        if self.eat("^") {
            self.deeper()?;
            let b = self.unary()?;
            self.depth -= 1;
            return Ok(Ast::Bin(Op::Pow, Box::new(a), Box::new(b), at));
        }
        Ok(a)
    }

    fn postfix(&mut self) -> Result<Ast, Refusal> {
        let mut a = self.atom()?;
        loop {
            let at = self.at();
            if !self.eat(".") {
                return Ok(a);
            }
            let axis = match self.peek() {
                Some(Tok::Name(n)) if n == "x" => 0,
                Some(Tok::Name(n)) if n == "y" => 1,
                Some(Tok::Name(n)) if n == "z" => 2,
                _ => return Err(syntax(self.at(), "after . comes x, y or z")),
            };
            self.pos += 1;
            a = Ast::Comp(Box::new(a), axis, at);
        }
    }

    fn atom(&mut self) -> Result<Ast, Refusal> {
        let at = self.at();
        let Some(tok) = self.peek().cloned() else {
            return Err(syntax(at, "the formula ends too early"));
        };
        self.pos += 1;
        match tok {
            Tok::Num(v) => Ok(Ast::Num(v)),
            Tok::Quoted(name) => Ok(Ast::Name {
                name,
                quoted: true,
                at,
            }),
            Tok::Name(name) => {
                if !self.eat("(") {
                    return Ok(Ast::Name {
                        name,
                        quoted: false,
                        at,
                    });
                }
                self.deeper()?;
                let mut args = Vec::new();
                if !self.eat(")") {
                    loop {
                        args.push(self.compare()?);
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",", &format!("{name}( … )"))?;
                    }
                }
                self.depth -= 1;
                Ok(Ast::Call(name, args, at))
            }
            Tok::Sym("(") => {
                self.deeper()?;
                let a = self.compare()?;
                if !self.eat(")") {
                    return Err(syntax(
                        self.at(),
                        format!("the ( of column {at} is not closed"),
                    ));
                }
                self.depth -= 1;
                Ok(a)
            }
            Tok::Sym("[") => {
                self.deeper()?;
                let x = self.compare()?;
                self.expect(",", "a vector [a, b, c]")?;
                let y = self.compare()?;
                self.expect(",", "a vector [a, b, c]")?;
                let z = self.compare()?;
                self.expect("]", "a vector [a, b, c]")?;
                self.depth -= 1;
                Ok(Ast::Vector(Box::new([x, y, z])))
            }
            t => Err(syntax(at, format!("{} is not expected here", show(&t)))),
        }
    }
}

// ---------------------------------------------------------------------
// Finding the names and checking the types.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    S,
    V,
}

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::S => "a scalar",
            Ty::V => "a vector",
        }
    }

    fn components(self) -> usize {
        match self {
            Ty::S => 1,
            Ty::V => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum F1 {
    Sqrt,
    Abs,
    Exp,
    Log,
    Log10,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Sign,
    Floor,
    Ceil,
}

impl F1 {
    fn of(name: &str) -> Option<F1> {
        Some(match name {
            "sqrt" => F1::Sqrt,
            "abs" => F1::Abs,
            "exp" => F1::Exp,
            "log" => F1::Log,
            "log10" => F1::Log10,
            "sin" => F1::Sin,
            "cos" => F1::Cos,
            "tan" => F1::Tan,
            "asin" => F1::Asin,
            "acos" => F1::Acos,
            "atan" => F1::Atan,
            "sinh" => F1::Sinh,
            "cosh" => F1::Cosh,
            "tanh" => F1::Tanh,
            "sign" => F1::Sign,
            "floor" => F1::Floor,
            "ceil" => F1::Ceil,
            _ => return None,
        })
    }

    fn apply(self, v: f64) -> f64 {
        match self {
            F1::Sqrt => v.sqrt(),
            F1::Abs => v.abs(),
            F1::Exp => v.exp(),
            F1::Log => v.ln(),
            F1::Log10 => v.log10(),
            F1::Sin => v.sin(),
            F1::Cos => v.cos(),
            F1::Tan => v.tan(),
            F1::Asin => v.asin(),
            F1::Acos => v.acos(),
            F1::Atan => v.atan(),
            F1::Sinh => v.sinh(),
            F1::Cosh => v.cosh(),
            F1::Tanh => v.tanh(),
            F1::Sign => {
                if v.is_nan() || v == 0.0 {
                    v
                } else {
                    v.signum()
                }
            }
            F1::Floor => v.floor(),
            F1::Ceil => v.ceil(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum F2 {
    Atan2,
    Min,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum D {
    Grad,
    Curl,
    Div,
}

/// The formula with its names found: what is evaluated.
#[derive(Clone, Debug)]
enum Ex {
    Num(f64),
    /// A field, by its slot in [`Plan::fields`].
    Field(usize),
    Coord(usize),
    Pos,
    Neg(Box<Ex>),
    Bin(Op, Box<Ex>, Box<Ex>),
    Comp(Box<Ex>, usize),
    Vector(Box<[Ex; 3]>),
    F1(F1, Box<Ex>),
    F2(F2, Box<Ex>, Box<Ex>),
    Mag(Box<Ex>),
    Dot(Box<Ex>, Box<Ex>),
    Cross(Box<Ex>, Box<Ex>),
    If(Box<Ex>, Box<Ex>, Box<Ex>),
    /// A derivative taken beforehand, by its slot in [`Plan::diffs`].
    Diff(usize),
}

impl Ex {
    /// The field slots it reads, and whether it holds a derivative.
    fn uses(&self, slots: &mut Vec<usize>) -> bool {
        match self {
            Ex::Num(_) | Ex::Coord(_) | Ex::Pos => false,
            Ex::Field(k) => {
                if !slots.contains(k) {
                    slots.push(*k);
                }
                false
            }
            Ex::Diff(_) => true,
            Ex::Neg(a) | Ex::Comp(a, _) | Ex::F1(_, a) | Ex::Mag(a) => a.uses(slots),
            Ex::Bin(_, a, b) | Ex::F2(_, a, b) | Ex::Dot(a, b) | Ex::Cross(a, b) => {
                let x = a.uses(slots);
                b.uses(slots) || x
            }
            Ex::If(a, b, c) => {
                let x = a.uses(slots);
                let y = b.uses(slots);
                c.uses(slots) || x || y
            }
            Ex::Vector(v) => {
                let mut any = false;
                for e in v.iter() {
                    any |= e.uses(slots);
                }
                any
            }
        }
    }
}

/// A derivative of the formula: its kind and what it is taken of.
#[derive(Clone, Debug)]
struct Diff {
    kind: D,
    arg: Ex,
}

/// A formula ready to evaluate.
#[derive(Clone, Debug)]
struct Plan {
    root: Ex,
    ty: Ty,
    /// The fields read, by slot.
    fields: Vec<String>,
    /// The derivatives, inner ones first.
    diffs: Vec<Diff>,
}

fn plan(ast: &Ast, set: &FieldSet) -> Result<Plan, Refusal> {
    let mut p = Plan {
        root: Ex::Num(0.0),
        ty: Ty::S,
        fields: Vec::new(),
        diffs: Vec::new(),
    };
    let (root, ty) = resolve(ast, set, &mut p)?;
    p.root = root;
    p.ty = ty;
    Ok(p)
}

fn type_error(at: usize, why: impl std::fmt::Display) -> Refusal {
    Refusal::Type(format!("{why} (column {at})"))
}

fn field_list(set: &FieldSet) -> String {
    let names = set.names();
    if names.is_empty() {
        return "the result has no fields".into();
    }
    format!("the fields are {}", names.join(", "))
}

fn resolve(ast: &Ast, set: &FieldSet, p: &mut Plan) -> Result<(Ex, Ty), Refusal> {
    Ok(match ast {
        Ast::Num(v) => (Ex::Num(*v), Ty::S),
        Ast::Name { name, quoted, at } => {
            if let Some(f) = set.get(name) {
                let ty = match f.components {
                    1 => Ty::S,
                    3 => Ty::V,
                    n => {
                        return Err(type_error(
                            *at,
                            format!(
                                "{name} has {n} components; formulas take scalars and vectors of 3"
                            ),
                        ));
                    }
                };
                let slot = match p.fields.iter().position(|n| n == name) {
                    Some(k) => k,
                    None => {
                        p.fields.push(name.clone());
                        p.fields.len() - 1
                    }
                };
                return Ok((Ex::Field(slot), ty));
            }
            let builtin = match name.as_str() {
                _ if *quoted => None,
                "x" => Some((Ex::Coord(0), Ty::S)),
                "y" => Some((Ex::Coord(1), Ty::S)),
                "z" => Some((Ex::Coord(2), Ty::S)),
                "pos" => Some((Ex::Pos, Ty::V)),
                "pi" => Some((Ex::Num(std::f64::consts::PI), Ty::S)),
                _ => None,
            };
            builtin.ok_or_else(|| {
                Refusal::Unknown(format!(
                    "no field {name} (column {at}); {}; built in are x, y, z, pos and pi",
                    field_list(set)
                ))
            })?
        }
        Ast::Neg(a) => {
            let (a, t) = resolve(a, set, p)?;
            (Ex::Neg(Box::new(a)), t)
        }
        Ast::Bin(op, a, b, at) => {
            let (a, ta) = resolve(a, set, p)?;
            let (b, tb) = resolve(b, set, p)?;
            let ty = match (op, ta, tb) {
                (Op::Add | Op::Sub, x, y) if x == y => x,
                (Op::Mul, Ty::S, t) | (Op::Mul, t, Ty::S) => t,
                (Op::Div, t, Ty::S) => t,
                (_, Ty::S, Ty::S) => Ty::S,
                (Op::Mul, Ty::V, Ty::V) => {
                    return Err(type_error(
                        *at,
                        "* of two vectors: say dot(a, b) or cross(a, b)",
                    ));
                }
                _ => {
                    return Err(type_error(
                        *at,
                        format!(
                            "{} takes {}; here {} and {}",
                            op.symbol(),
                            match op {
                                Op::Add | Op::Sub => "two scalars or two vectors",
                                Op::Div => "a scalar or a vector by a scalar",
                                _ => "two scalars",
                            },
                            ta.name(),
                            tb.name()
                        ),
                    ));
                }
            };
            (Ex::Bin(*op, Box::new(a), Box::new(b)), ty)
        }
        Ast::Comp(a, axis, at) => {
            let (a, t) = resolve(a, set, p)?;
            if t != Ty::V {
                return Err(type_error(
                    *at,
                    ".x, .y and .z are the components of a vector; this is a scalar",
                ));
            }
            (Ex::Comp(Box::new(a), *axis), Ty::S)
        }
        Ast::Vector(v) => {
            let mut out = Vec::with_capacity(3);
            for e in v.iter() {
                let (e, t) = resolve(e, set, p)?;
                if t != Ty::S {
                    return Err(Refusal::Type(
                        "each component of a vector [a, b, c] is a scalar".into(),
                    ));
                }
                out.push(e);
            }
            let [x, y, z]: [Ex; 3] = out
                .try_into()
                .map_err(|_| Refusal::Type("a vector has three components".into()))?;
            (Ex::Vector(Box::new([x, y, z])), Ty::V)
        }
        Ast::Call(name, args, at) => call(name, args, *at, set, p)?,
    })
}

fn call(
    name: &str,
    args: &[Ast],
    at: usize,
    set: &FieldSet,
    p: &mut Plan,
) -> Result<(Ex, Ty), Refusal> {
    let Some(f) = FUNCTIONS.iter().find(|f| f.name == name) else {
        return Err(Refusal::Unknown(format!(
            "no function {name} (column {at}); there are {}",
            FUNCTIONS
                .iter()
                .map(|f| f.name)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    };
    let want: Vec<&str> = f.args.split(", ").collect();
    if args.len() != want.len() {
        return Err(type_error(
            at,
            format!(
                "{name} takes {} argument{} ({name}({})); here {}",
                want.len(),
                if want.len() == 1 { "" } else { "s" },
                f.args,
                args.len()
            ),
        ));
    }
    let mut got = Vec::with_capacity(args.len());
    for a in args {
        got.push(resolve(a, set, p)?);
    }
    let mut t_of: Option<Ty> = None;
    for (k, ((_, ty), w)) in got.iter().zip(&want).enumerate() {
        let ok = match *w {
            "s" => *ty == Ty::S,
            "v" => *ty == Ty::V,
            _ => match t_of {
                None => {
                    t_of = Some(*ty);
                    true
                }
                Some(t) => t == *ty,
            },
        };
        if !ok {
            let hint = match (name, *ty) {
                ("abs", Ty::V) => "; mag(v) is the magnitude of a vector",
                ("grad", Ty::V) => "; grad takes a scalar: grad(U.x), or curl(U) or div(U)",
                (_, Ty::V) => "; mag(v) or v.x make a scalar of it",
                _ => "",
            };
            return Err(type_error(
                at,
                format!(
                    "{name}({}): argument {} is {}{hint}",
                    f.args,
                    k + 1,
                    ty.name()
                ),
            ));
        }
    }
    let ty = match f.gives {
        "s" => Ty::S,
        "v" => Ty::V,
        _ => t_of.unwrap_or(Ty::S),
    };
    let mut it = got.into_iter().map(|(e, _)| Box::new(e));
    let mut next = || it.next().unwrap_or_else(|| Box::new(Ex::Num(f64::NAN)));
    let ex = if let Some(op) = F1::of(name) {
        Ex::F1(op, next())
    } else {
        match name {
            "atan2" => Ex::F2(F2::Atan2, next(), next()),
            "min" => Ex::F2(F2::Min, next(), next()),
            "max" => Ex::F2(F2::Max, next(), next()),
            "if" => Ex::If(next(), next(), next()),
            "mag" => Ex::Mag(next()),
            "dot" => Ex::Dot(next(), next()),
            "cross" => Ex::Cross(next(), next()),
            "grad" | "curl" | "div" => {
                let kind = match name {
                    "grad" => D::Grad,
                    "curl" => D::Curl,
                    _ => D::Div,
                };
                p.diffs.push(Diff { kind, arg: *next() });
                Ex::Diff(p.diffs.len() - 1)
            }
            _ => {
                return Err(Refusal::Unknown(format!(
                    "no function {name} (column {at})"
                )));
            }
        }
    };
    Ok((ex, ty))
}

// ---------------------------------------------------------------------
// Evaluating.

#[derive(Clone, Copy, Debug)]
enum Val {
    S(f64),
    V([f64; 3]),
}

impl Val {
    fn s(self) -> f64 {
        match self {
            Val::S(v) => v,
            Val::V(_) => f64::NAN,
        }
    }

    fn v(self) -> [f64; 3] {
        match self {
            Val::V(v) => v,
            Val::S(_) => [f64::NAN; 3],
        }
    }
}

/// A column of values: `k` per entity, entity after entity.
#[derive(Clone, Debug)]
struct Col<'a> {
    k: usize,
    values: Cow<'a, [f64]>,
}

impl Col<'_> {
    fn at(&self, i: usize) -> Val {
        if self.k == 3 {
            match self.values.get(i * 3..i * 3 + 3) {
                Some(v) => Val::V([v[0], v[1], v[2]]),
                None => Val::V([f64::NAN; 3]),
            }
        } else {
            Val::S(self.values.get(i).copied().unwrap_or(f64::NAN))
        }
    }
}

/// What a formula reads at one place: its fields, where each entity
/// sits, and the derivatives taken so far (in the cells only).
struct Cols<'a> {
    n: usize,
    fields: Vec<Option<Col<'a>>>,
    places: &'a [[f64; 3]],
    diffs: Vec<Col<'static>>,
}

fn eval(e: &Ex, i: usize, c: &Cols) -> Val {
    match e {
        Ex::Num(v) => Val::S(*v),
        Ex::Field(k) => match c.fields.get(*k).and_then(Option::as_ref) {
            Some(col) => col.at(i),
            None => Val::S(f64::NAN),
        },
        Ex::Coord(a) => Val::S(c.places.get(i).map_or(f64::NAN, |p| p[*a])),
        Ex::Pos => Val::V(c.places.get(i).copied().unwrap_or([f64::NAN; 3])),
        Ex::Neg(a) => match eval(a, i, c) {
            Val::S(v) => Val::S(-v),
            Val::V(v) => Val::V(v.map(|x| -x)),
        },
        Ex::Bin(op, a, b) => {
            let (a, b) = (eval(a, i, c), eval(b, i, c));
            match (op, a, b) {
                (_, Val::S(x), Val::S(y)) => Val::S(match op {
                    Op::Add => x + y,
                    Op::Sub => x - y,
                    Op::Mul => x * y,
                    Op::Div => x / y,
                    Op::Pow => x.powf(y),
                    Op::Lt => truth(x < y, x, y),
                    Op::Le => truth(x <= y, x, y),
                    Op::Gt => truth(x > y, x, y),
                    Op::Ge => truth(x >= y, x, y),
                }),
                (Op::Add, Val::V(x), Val::V(y)) => Val::V([0, 1, 2].map(|d| x[d] + y[d])),
                (Op::Sub, Val::V(x), Val::V(y)) => Val::V([0, 1, 2].map(|d| x[d] - y[d])),
                (Op::Mul, Val::S(s), Val::V(v)) | (Op::Mul, Val::V(v), Val::S(s)) => {
                    Val::V(v.map(|x| x * s))
                }
                (Op::Div, Val::V(v), Val::S(s)) => Val::V(v.map(|x| x / s)),
                _ => Val::S(f64::NAN),
            }
        }
        Ex::Comp(a, axis) => Val::S(eval(a, i, c).v()[*axis]),
        Ex::Vector(v) => Val::V([0, 1, 2].map(|d| eval(&v[d], i, c).s())),
        Ex::F1(f, a) => Val::S(f.apply(eval(a, i, c).s())),
        Ex::F2(f, a, b) => {
            let (x, y) = (eval(a, i, c).s(), eval(b, i, c).s());
            Val::S(match f {
                F2::Atan2 => x.atan2(y),
                // NaN wins: no value in, no value out.
                F2::Min if x.is_nan() || y.is_nan() => f64::NAN,
                F2::Max if x.is_nan() || y.is_nan() => f64::NAN,
                F2::Min => x.min(y),
                F2::Max => x.max(y),
            })
        }
        Ex::Mag(a) => match eval(a, i, c) {
            Val::S(v) => Val::S(v.abs()),
            Val::V(v) => Val::S(v.iter().map(|x| x * x).sum::<f64>().sqrt()),
        },
        Ex::Dot(a, b) => {
            let (x, y) = (eval(a, i, c).v(), eval(b, i, c).v());
            Val::S(x[0] * y[0] + x[1] * y[1] + x[2] * y[2])
        }
        Ex::Cross(a, b) => {
            let (x, y) = (eval(a, i, c).v(), eval(b, i, c).v());
            Val::V([
                x[1] * y[2] - x[2] * y[1],
                x[2] * y[0] - x[0] * y[2],
                x[0] * y[1] - x[1] * y[0],
            ])
        }
        Ex::If(cond, a, b) => {
            let k = eval(cond, i, c).s();
            if k.is_nan() {
                match eval(a, i, c) {
                    Val::S(_) => Val::S(f64::NAN),
                    Val::V(_) => Val::V([f64::NAN; 3]),
                }
            } else if k != 0.0 {
                eval(a, i, c)
            } else {
                eval(b, i, c)
            }
        }
        Ex::Diff(k) => match c.diffs.get(*k) {
            Some(col) => col.at(i),
            None => Val::S(f64::NAN),
        },
    }
}

/// 1 or 0, no value when either side has none.
fn truth(b: bool, x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if b {
        1.0
    } else {
        0.0
    }
}

/// `e` (of type `ty`) at every entity of `c`, on all threads; values
/// that are not finite become NaN (no value).
fn column(e: &Ex, ty: Ty, c: &Cols) -> Vec<f64> {
    let k = ty.components();
    murmel_mesh::par::map(c.n, 8192, |r| {
        let mut out = Vec::with_capacity(r.len() * k);
        for i in r {
            match (eval(e, i, c), ty) {
                (Val::S(v), Ty::S) => out.push(v),
                (Val::V(v), Ty::V) => out.extend(v),
                _ => out.extend(std::iter::repeat_n(f64::NAN, k)),
            }
        }
        for v in &mut out {
            if !v.is_finite() {
                *v = f64::NAN;
            }
        }
        out
    })
    .concat()
}

const PLACES: [Location; 3] = [Location::Cell, Location::Point, Location::Boundary];

fn place_index(l: Location) -> usize {
    match l {
        Location::Cell => 0,
        Location::Point => 1,
        Location::Boundary => 2,
    }
}

fn place_name(l: Location) -> &'static str {
    match l {
        Location::Cell => "in the cells",
        Location::Point => "at the nodes",
        Location::Boundary => "on the boundary faces",
    }
}

fn count(mesh: &Mesh, l: Location) -> usize {
    match l {
        Location::Cell => mesh.cell_count(),
        Location::Point => mesh.points.len(),
        Location::Boundary => mesh.boundary.len(),
    }
}

/// Field `name` of `set` at `l`, fitting the mesh; in the cells, a
/// node field as the mean of each cell's nodes when the file has none
/// there (and `means`).
fn field_at<'a>(
    set: &'a FieldSet,
    mesh: &Mesh,
    name: &str,
    l: Location,
    means: bool,
) -> Option<Col<'a>> {
    if let Some(f) = set.at(name, l).filter(|f| FieldSet::fits(f, mesh)) {
        return Some(Col {
            k: f.components.max(1),
            values: Cow::Borrowed(&f.values),
        });
    }
    if l == Location::Cell && means {
        let f = set
            .at(name, Location::Point)
            .filter(|f| FieldSet::fits(f, mesh))?;
        let k = f.components.max(1);
        return Some(Col {
            k,
            values: Cow::Owned(set.cell_means(mesh, &f.values, k)),
        });
    }
    None
}

/// Whether field `name` is at `l` exactly.
fn is_at(set: &FieldSet, mesh: &Mesh, name: &str, l: Location) -> bool {
    set.at(name, l).is_some_and(|f| FieldSet::fits(f, mesh))
}

/// What `p` reads at `l`; nothing when `l` is not `needed`.
fn columns<'a>(set: &'a FieldSet, mesh: &'a Mesh, p: &Plan, l: Location, needed: bool) -> Cols<'a> {
    if !needed {
        return Cols {
            n: 0,
            fields: Vec::new(),
            places: &[],
            diffs: Vec::new(),
        };
    }
    Cols {
        n: count(mesh, l),
        fields: p
            .fields
            .iter()
            .map(|n| field_at(set, mesh, n, l, true))
            .collect(),
        places: set.places(mesh, l),
        diffs: Vec::new(),
    }
}

/// Take derivative `d` in the cells: of its argument at the nodes when
/// all it reads is there (each cell fitted to its nodes), else of its
/// argument in the cells (with its values on the boundary faces when
/// all it reads is there too).
fn derivative(
    set: &FieldSet,
    mesh: &Mesh,
    p: &Plan,
    d: &Diff,
    cells: &Cols,
    nodes: &Cols,
    faces: &Cols,
) -> Col<'static> {
    let mut slots = Vec::new();
    let nested = d.arg.uses(&mut slots);
    let all_at = |l: Location| {
        !nested
            && slots
                .iter()
                .all(|&k| p.fields.get(k).is_some_and(|n| is_at(set, mesh, n, l)))
    };
    let ty = match d.kind {
        D::Grad => Ty::S,
        D::Curl | D::Div => Ty::V,
    };
    let k = ty.components();
    // One gradient per component of the argument, in every cell.
    let grads: Vec<Vec<[f64; 3]>> = if all_at(Location::Point) && mesh.points.len() == nodes.n {
        let v = column(&d.arg, ty, nodes);
        (0..k)
            .map(|j| {
                let comp: Vec<f64> = v.iter().skip(j).step_by(k).copied().collect();
                set.node_fit_gradient(mesh, &comp)
            })
            .collect()
    } else {
        let v = column(&d.arg, ty, cells);
        let b = all_at(Location::Boundary).then(|| column(&d.arg, ty, faces));
        (0..k)
            .map(|j| {
                let comp: Vec<f64> = v.iter().skip(j).step_by(k).copied().collect();
                let bc: Option<Vec<f64>> = b
                    .as_ref()
                    .map(|b| b.iter().skip(j).step_by(k).copied().collect());
                set.cell_gradient(mesh, &comp, bc.as_deref())
            })
            .collect()
    };
    let n = mesh.cell_count();
    let g = |i: usize, c: usize| {
        grads
            .get(i)
            .and_then(|g| g.get(c))
            .copied()
            .unwrap_or([f64::NAN; 3])
    };
    let finite = |v: f64| if v.is_finite() { v } else { f64::NAN };
    match d.kind {
        D::Grad => Col {
            k: 3,
            values: Cow::Owned((0..n).flat_map(|c| g(0, c).map(finite)).collect()),
        },
        D::Curl => Col {
            k: 3,
            values: Cow::Owned(
                (0..n)
                    .flat_map(|c| {
                        let m = [g(0, c), g(1, c), g(2, c)];
                        crate::derive::vorticity(m).map(finite)
                    })
                    .collect(),
            ),
        },
        D::Div => Col {
            k: 1,
            values: Cow::Owned(
                (0..n)
                    .map(|c| finite(g(0, c)[0] + g(1, c)[1] + g(2, c)[2]))
                    .collect(),
            ),
        },
    }
}

/// The fields `f` makes on `mesh` from the fields of `set`, named
/// `f.name`, at each place the module notes say.
pub fn compute(mesh: &Mesh, set: &FieldSet, f: &Formula) -> Result<Vec<Field>, Refusal> {
    check(f)?;
    if set.is_empty() {
        return Err(Refusal::NoFields);
    }
    if set.fields().iter().any(|x| x.name == f.name) && set.maker(&f.name).is_none() {
        return Err(Refusal::InFile(f.name.clone()));
    }
    let ast = parse(&f.formula)?;
    let p = plan(&ast, set)?;
    if p.fields.contains(&f.name) {
        return Err(Refusal::Cycle(format!("{} reads itself", f.name)));
    }
    // Every field read must fit the mesh somewhere.
    for n in &p.fields {
        if !PLACES.iter().any(|&l| is_at(set, mesh, n, l)) {
            return Err(Refusal::Type(format!(
                "{n} does not have a value for every cell, node or face"
            )));
        }
    }
    let exact: Vec<Location> = PLACES
        .into_iter()
        .filter(|&l| {
            (l == Location::Cell || p.diffs.is_empty())
                && p.fields.iter().all(|n| is_at(set, mesh, n, l))
        })
        .collect();
    let places = if !exact.is_empty() {
        exact
    } else if p
        .fields
        .iter()
        .all(|n| is_at(set, mesh, n, Location::Cell) || is_at(set, mesh, n, Location::Point))
    {
        vec![Location::Cell]
    } else {
        let wheres: Vec<String> = p
            .fields
            .iter()
            .map(|n| {
                let at: Vec<&str> = PLACES
                    .into_iter()
                    .filter(|&l| is_at(set, mesh, n, l))
                    .map(place_name)
                    .collect();
                format!("{n} is {}", at.join(" and "))
            })
            .collect();
        return Err(Refusal::Type(format!(
            "no place has every field the formula reads{}: {}",
            if p.diffs.is_empty() {
                ""
            } else {
                " (a derivative is taken in the cells)"
            },
            wheres.join("; ")
        )));
    };
    // The derivatives may read every place; the rest only where the
    // result goes.
    let mut cols: [Cols; 3] =
        PLACES.map(|l| columns(set, mesh, &p, l, !p.diffs.is_empty() || places.contains(&l)));
    for d in &p.diffs {
        let [cells, nodes, faces] = &cols;
        let col = derivative(set, mesh, &p, d, cells, nodes, faces);
        cols[0].diffs.push(col);
    }
    let unit = f.unit.clone().filter(|u| !u.trim().is_empty());
    Ok(places
        .into_iter()
        .map(|l| Field {
            name: f.name.clone(),
            unit: unit.clone(),
            location: l,
            components: p.ty.components(),
            values: column(&p.root, p.ty, &cols[place_index(l)]),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmel_mesh::field::Results;

    /// The polyhedral fixture (cells of every shape, walls around).
    fn mesh() -> Mesh {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/openfoam/dual_polyhedra");
        murmel_mesh::read(&p).unwrap()
    }

    fn set_of(fields: Vec<Field>) -> FieldSet {
        FieldSet::new(Results {
            fields,
            ..Default::default()
        })
    }

    fn cell_field(mesh: &Mesh, name: &str, f: impl Fn([f64; 3]) -> Vec<f64>) -> Field {
        let mut values = Vec::new();
        let mut components = 1;
        for c in 0..mesh.cell_count() {
            let v = f(crate::inspect::cell_centroid(mesh, c));
            components = v.len();
            values.extend(v);
        }
        Field {
            name: name.into(),
            unit: None,
            location: Location::Cell,
            components,
            values,
        }
    }

    fn formula(name: &str, text: &str) -> Formula {
        Formula {
            name: name.into(),
            formula: text.into(),
            unit: None,
        }
    }

    /// Invariant: the grammar reads precedence, associativity, numbers,
    /// quoted names, components and vectors as the module notes say,
    /// and says where a formula goes wrong.
    #[test]
    fn grammar() {
        let mesh = mesh();
        let set = set_of(vec![
            cell_field(&mesh, "p", |_| vec![2.0]),
            cell_field(&mesh, "U", |_| vec![1.0, 2.0, 3.0]),
            cell_field(&mesh, "p rgh", |_| vec![5.0]),
        ]);
        let one = |text: &str| -> Result<Vec<f64>, Refusal> {
            let out = compute(&mesh, &set, &formula("r", text))?;
            Ok(out[0].values[..out[0].components].to_vec())
        };
        let cases: &[(&str, &[f64])] = &[
            ("1 + 2 * 3", &[7.0]),
            ("(1 + 2) * 3", &[9.0]),
            ("2 ^ 3 ^ 2", &[512.0]),
            ("-2 ^ 2", &[-4.0]),
            ("2 ^ -1", &[0.5]),
            ("8 / 4 / 2", &[1.0]),
            ("1e-3 * 2E+3", &[2.0]),
            (".5 + 1.", &[1.5]),
            ("p * \"p rgh\"", &[10.0]),
            ("'p rgh' - p", &[3.0]),
            ("U.y", &[2.0]),
            ("[p, 0, -p]", &[2.0, 0.0, -2.0]),
            ("[p, 0, -p].z", &[-2.0]),
            ("2 * U - U", &[1.0, 2.0, 3.0]),
            ("U / 2", &[0.5, 1.0, 1.5]),
            ("mag([3, 4, 0])", &[5.0]),
            ("dot(U, U)", &[14.0]),
            ("cross([1, 0, 0], [0, 1, 0])", &[0.0, 0.0, 1.0]),
            ("min(p, 1) + max(p, 1)", &[3.0]),
            ("p > 1", &[1.0]),
            ("p <= 1", &[0.0]),
            ("if(p > 1, U, -U).x", &[1.0]),
            ("atan2(1, 1) * 4 / pi", &[1.0]),
            ("sqrt(16) + abs(-1) + sign(-3)", &[4.0]),
            ("log10(1000) + log(exp(2))", &[5.0]),
            ("floor(2.7) + ceil(2.1)", &[5.0]),
        ];
        for (text, want) in cases {
            let got = one(text).unwrap_or_else(|e| panic!("{text}: {}", e.message()));
            assert_eq!(got.len(), want.len(), "{text}");
            for (g, w) in got.iter().zip(want.iter()) {
                assert!((g - w).abs() < 1e-12, "{text}: {g} != {w}");
            }
        }
        let refused: &[(&str, &str)] = &[
            ("1 +", "FIELD011"),
            ("(1 + 2", "FIELD011"),
            ("1 2", "FIELD011"),
            ("U.w", "FIELD011"),
            ("p $ 2", "FIELD011"),
            ("'p", "FIELD011"),
            ("1 < p < 3", "FIELD011"),
            ("", "FIELD011"),
            ("q", "FIELD012"),
            ("'x'", "FIELD012"),
            ("foo(p)", "FIELD012"),
            ("U * U", "FIELD013"),
            ("U + p", "FIELD013"),
            ("p / U", "FIELD013"),
            ("sqrt(U)", "FIELD013"),
            ("p.x", "FIELD013"),
            ("min(p)", "FIELD013"),
            ("[U, 0, 0]", "FIELD013"),
            ("if(U, p, p)", "FIELD013"),
            ("if(p, U, p)", "FIELD013"),
            ("grad(U)", "FIELD013"),
        ];
        for (text, code) in refused {
            match one(text) {
                Err(e) => assert_eq!(e.code(), *code, "{text}: {}", e.message()),
                Ok(v) => panic!("{text} gave {v:?}"),
            }
        }
        // Where it went wrong is said by column.
        let e = one("p + (U").err().map(|e| e.message()).unwrap_or_default();
        assert!(e.contains("column 5"), "{e}");
        let e = one("p + qq").err().map(|e| e.message()).unwrap_or_default();
        assert!(e.contains("column 5") && e.contains("p, U"), "{e}");
        // Deep nesting is refused, not a stack overflow.
        let deep = format!("{}1{}", "(".repeat(500), ")".repeat(500));
        assert_eq!(one(&deep).err().map(|e| e.code()), Some("FIELD011"));
        let long = "1+".repeat(MAX_FORMULA) + "1";
        assert_eq!(one(&long).err().map(|e| e.code()), Some("FIELD011"));
    }

    /// Invariant: a formula's own field name follows the rules, and a
    /// value that is not finite is no value.
    #[test]
    fn names_and_no_values() {
        let mesh = mesh();
        let mid = (0..mesh.cell_count())
            .map(|c| crate::inspect::cell_centroid(&mesh, c)[0])
            .sum::<f64>()
            / mesh.cell_count() as f64;
        let set = set_of(vec![cell_field(&mesh, "p", |x| vec![x[0] - mid])]);
        for bad in [
            "",
            "2p",
            "a b",
            "a-b",
            "vorticity",
            "q_criterion",
            &"a".repeat(65),
        ] {
            assert_eq!(
                compute(&mesh, &set, &formula(bad, "1"))
                    .err()
                    .map(|e| e.code()),
                Some("FIELD014"),
                "{bad:?}"
            );
        }
        assert_eq!(
            compute(&mesh, &set, &formula("p", "1"))
                .err()
                .map(|e| e.code()),
            Some("FIELD008")
        );
        let out = compute(&mesh, &set, &formula("r", "sqrt(p) + 1 / (p - p)")).unwrap_or_default();
        assert!(out[0].values.iter().all(|v| v.is_nan()));
        let out = compute(&mesh, &set, &formula("r", "sqrt(p)")).unwrap_or_default();
        let none = out[0].values.iter().filter(|v| v.is_nan()).count();
        let below = (0..mesh.cell_count())
            .filter(|&c| crate::inspect::cell_centroid(&mesh, c)[0] < mid)
            .count();
        assert!(below > 0 && none == below);
    }

    /// Invariant: grad, curl and div are exact for linear fields, in
    /// the cells, from cell fields and from node fields alike; a field
    /// at the nodes alone gives a result at the nodes, and one at the
    /// nodes and one in the cells a result in the cells.
    #[test]
    fn derivatives_and_places() {
        let mesh = mesh();
        let lin = |x: [f64; 3]| vec![2.0 * x[0] - 3.0 * x[1] + 0.5 * x[2]];
        // u = (y, -x, z): curl (0, 0, -2), div 1.
        let vel = |x: [f64; 3]| vec![x[1], -x[0], x[2]];
        let mut node_u = Field {
            name: "Un".into(),
            unit: None,
            location: Location::Point,
            components: 3,
            values: mesh.points.iter().flat_map(|&x| vel(x)).collect(),
        };
        let set = set_of(vec![
            cell_field(&mesh, "f", lin),
            cell_field(&mesh, "U", vel),
            node_u.clone(),
        ]);
        let close = |out: &[Field], want: &[f64]| {
            let f = &out[0];
            assert_eq!(f.location, Location::Cell);
            for i in 0..f.len() {
                for (j, w) in want.iter().enumerate() {
                    let v = f.values[i * f.components + j];
                    assert!((v - w).abs() < 1e-6, "{v} != {w}");
                }
            }
        };
        let run = |t: &str| compute(&mesh, &set, &formula("r", t)).unwrap_or_default();
        close(&run("grad(f)"), &[2.0, -3.0, 0.5]);
        close(&run("curl(U)"), &[0.0, 0.0, -2.0]);
        close(&run("curl(Un)"), &[0.0, 0.0, -2.0]);
        close(&run("div(U) + div(Un)"), &[2.0]);
        close(&run("grad(grad(f).x)"), &[0.0, 0.0, 0.0]);
        close(&run("grad(2 * x - 3 * y + z) + 0 * U"), &[2.0, -3.0, 1.0]);
        // A derivative's result is in the cells alone.
        assert_eq!(run("grad(f)").len(), 1);
        // A node field alone: the result is at the nodes.
        let out = run("mag(Un)");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].location, Location::Point);
        // A node field with a cell field: in the cells, the node field
        // as each cell's mean.
        let out = run("f + Un.x");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].location, Location::Cell);
        // Coordinates are where the values sit.
        let out = run("x + 0 * f");
        let c = crate::inspect::cell_centroid(&mesh, 3);
        assert!((out[0].values[3] - c[0]).abs() < 1e-12);
        // A field on the boundary faces alone cannot meet a cell field.
        node_u.location = Location::Boundary;
        node_u.values = vec![1.0; mesh.boundary.len() * 3];
        node_u.name = "Ub".into();
        let set = set_of(vec![cell_field(&mesh, "f", lin), node_u]);
        let e = compute(&mesh, &set, &formula("r", "f + Ub.x")).err();
        assert_eq!(e.as_ref().map(|e| e.code()), Some("FIELD013"));
        let out = compute(&mesh, &set, &formula("r", "Ub.x * 2")).unwrap_or_default();
        assert_eq!(out[0].location, Location::Boundary);
        assert!(out[0].values.iter().all(|v| *v == 2.0));
    }

    /// Invariant: what a formula reads is the fields it names, each
    /// once, a quoted built-in name included, never a built-in.
    #[test]
    fn what_it_reads() {
        let mesh = mesh();
        let set = set_of(vec![
            cell_field(&mesh, "p", |_| vec![1.0]),
            cell_field(&mesh, "x", |_| vec![1.0]),
        ]);
        assert_eq!(reads("p + p * y + x + pi", &set), vec!["p", "x"]);
        assert!(reads("p +", &set).is_empty());
    }
    /// Invariant: a formula's field joins the open result's fields; one
    /// made from another (or from a derived quantity) is made again when
    /// that one changes, in order; a formula that would read itself
    /// through another is refused; a field read by another cannot be
    /// removed; a session file brings them all back in order. Breaks
    /// when a field made from a changed one keeps its old values.
    #[test]
    fn formula_fields_in_the_session() {
        use crate::derive::{Made, Quantity, Recipe};
        use crate::session::Session;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
        let mut s = Session::new(crate::theme::Tokens::builtin().unwrap());
        s.open(&path).unwrap();
        let f = |name: &str, text: &str| Formula {
            name: name.into(),
            formula: text.into(),
            unit: None,
        };
        s.derive(Recipe::new(Quantity::Vorticity)).unwrap();
        assert_eq!(
            s.calculate(f("a", "2 * pressure")),
            Ok(vec!["a".to_string()])
        );
        assert_eq!(
            s.calculate(f("b", "a + mag(vorticity)")),
            Ok(vec!["b".to_string()])
        );
        assert_eq!(s.calculate(f("c", "b * 0 + 1")), Ok(vec!["c".to_string()]));
        let value = |s: &Session, n: &str, i: usize| {
            s.fields().unwrap().at(n, Location::Cell).unwrap().values[i]
        };
        let b0 = value(&s, "b", 5);
        // Making a again makes b and c again, after it.
        assert_eq!(
            s.calculate(f("a", "3 * pressure")),
            Ok(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
        let p5 = value(&s, "pressure", 5);
        assert!((value(&s, "b", 5) - (b0 + p5)).abs() < 1e-9 * b0.abs().max(1.0));
        let order: Vec<&str> = s.fields().unwrap().made().iter().map(Made::field).collect();
        assert_eq!(order, ["vorticity", "a", "b", "c"]);
        // a reading c, made from a: refused, nothing changed.
        assert_eq!(
            s.calculate(f("a", "c")).err().map(|e| e.code()),
            Some("FIELD015")
        );
        assert_eq!(
            s.calculate(f("a", "a + 1")).err().map(|e| e.code()),
            Some("FIELD015")
        );
        // A change that breaks b (a vector now) is refused.
        let e = s.calculate(f("a", "velocity")).err();
        assert_eq!(
            e.as_ref().map(|e| e.code()),
            Some("FIELD013"),
            "{:?}",
            e.map(|e| e.message())
        );
        assert!(s.fields().unwrap().get("a").is_some_and(|x| !x.is_vector()));
        // b reads a and vorticity: neither can go while it is there.
        assert_eq!(s.underive("a").err().map(|e| e.code()), Some("FIELD010"));
        assert_eq!(
            s.underive("vorticity").err().map(|e| e.code()),
            Some("FIELD010")
        );
        // Saved and loaded, in order.
        let file = std::env::temp_dir().join(format!("murmel-calc-{}.murmel", std::process::id()));
        let text = crate::session_file::SessionFile::of(&s, &file).to_json();
        let mut t = Session::new(crate::theme::Tokens::builtin().unwrap());
        t.open(&path).unwrap();
        let missing = t.restore(crate::session_file::SessionFile::parse(&text).unwrap());
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(t.fields().unwrap().made(), s.fields().unwrap().made());
        assert_eq!(value(&t, "b", 5), value(&s, "b", 5));
        // c, then b, then a go.
        for n in ["c", "b", "a"] {
            s.underive(n).unwrap();
        }
        assert_eq!(s.fields().unwrap().made().len(), 1);
    }
}
