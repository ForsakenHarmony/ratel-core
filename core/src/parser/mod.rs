#[macro_use]
mod macros;
mod error;
mod expression;
mod statement;
mod function;
mod nested;

use error::Error;
use arena::Arena;
use module::Module;

use self::error::ToError;
use self::nested::*;

use ast::{Loc, Ptr, Statement, List, ListBuilder, EmptyListBuilder};
use ast::{Parameter, ParameterKey, ParameterPtr, ParameterList, OperatorKind};
use ast::{Expression, ExpressionPtr, ExpressionList, Block, BlockPtr};
use ast::expression::BinaryExpression;
use lexer::{Lexer, Asi};
use lexer::Token::*;

pub trait Parse<'ast> {
    type Output;

    fn parse(&mut Parser<'ast>) -> Self::Output;
}

pub struct Parser<'ast> {
    arena: &'ast Arena,

    /// Lexer will produce tokens from the source
    lexer: Lexer<'ast>,

    /// Errors occurred during parsing
    errors: Vec<Error>,

    /// AST under construction
    body: List<'ast, Loc<Statement<'ast>>>,
}

impl<'ast> Parser<'ast> {
    pub fn new(source: &str, arena: &'ast Arena) -> Self {
        Parser {
            arena,
            lexer: Lexer::new(arena, source),
            errors: Vec::new(),
            body: List::empty(),
        }
    }

    fn error<T: ToError>(&mut self) -> T {
        let err = self.lexer.invalid_token();

        self.errors.push(err);

        T::to_error()
    }

    #[inline]
    fn asi(&mut self) -> Asi {
        self.lexer.asi()
    }

    #[inline]
    fn loc(&self) -> (u32, u32) {
        self.lexer.loc()
    }

    #[inline]
    fn in_loc<T>(&self, item: T) -> Loc<T> {
        let (start, end) = self.loc();

        Loc::new(start, end, item)
    }

    #[inline]
    fn alloc<T>(&mut self, val: T) -> Ptr<'ast, T> where
        T: Copy,
    {
        Ptr::new(self.arena.alloc(val.into()))
    }

    #[inline]
    fn alloc_in_loc<T, I>(&mut self, item: I) -> Ptr<'ast, Loc<T>> where
        T: Copy,
        I: Into<T>,
    {
        let node = self.in_loc(item.into());
        self.alloc(node)
    }

    #[inline]
    fn alloc_at_loc<T, I>(&mut self, start: u32, end: u32, item: I) -> Ptr<'ast, Loc<T>> where
        T: Copy,
        I: Into<T>,
    {
        self.alloc(Loc::new(start, end, item.into()))
    }

    #[inline]
    fn parse(&mut self) {
        if self.lexer.token == EndOfProgram {
            return;
        }

        let statement = self.statement();
        let mut builder = ListBuilder::new(self.arena, statement);

        while self.lexer.token != EndOfProgram {
            builder.push(self.statement());
        }

        self.body = builder.into_list()
    }

    #[inline]
    fn block<I>(&mut self) -> BlockPtr<'ast, I> where
        I: Parse<'ast, Output = Ptr<'ast, Loc<I>>> + Copy
    {
        let start = match self.lexer.token {
            BraceOpen => self.lexer.start_then_consume(),
            _         => return self.error(),
        };
        let block = self.raw_block();
        let end   = self.lexer.end_then_consume();

        self.alloc_at_loc(start, end, block)
    }

    /// Same as above, but assumes that the opening brace has already been checked
    #[inline]
    fn unchecked_block<I>(&mut self) -> BlockPtr<'ast, I> where
        I: Parse<'ast, Output = Ptr<'ast, Loc<I>>> + Copy
    {
        let start = self.lexer.start_then_consume();
        let block = self.raw_block();
        let end   = self.lexer.end_then_consume();

        self.alloc_at_loc(start, end, block)
    }

    #[inline]
    fn raw_block<I>(&mut self) -> Block<'ast, I> where
        I: Parse<'ast, Output = Ptr<'ast, Loc<I>>> + Copy
    {
        if self.lexer.token == BraceClose {
            return Block { body: List::empty() };
        }

        let statement = I::parse(self);
        let mut builder = ListBuilder::new(self.arena, statement);

        while self.lexer.token != BraceClose {
            builder.push(I::parse(self));
        }

        Block { body: builder.into_list() }
    }

    #[inline]
    fn param_from_expression(&mut self, expression: ExpressionPtr<'ast>) -> ParameterPtr<'ast> {
        let (key, value) = match expression.item {
            Expression::Binary(BinaryExpression {
                operator: OperatorKind::Assign,
                left,
                right,
            }) => (left, Some(right)),
            _  => (expression, None)
        };

        let key = match key.item {
            Expression::Identifier(ident) => ParameterKey::Identifier(ident),
            // TODO: ParameterKey::Pattern
            _ => return self.error()
        };

        self.alloc(Loc::new(expression.start, expression.end, Parameter {
            key,
            value
        }))
    }

    #[inline]
    fn params_from_expressions(&mut self, expressions: ExpressionList<'ast>) -> ParameterList<'ast> {
        let mut expressions = expressions.ptr_iter();

        let mut builder = match expressions.next() {
            Some(&expression) => {
                let param = self.param_from_expression(expression);

                ListBuilder::new(self.arena, param)
            },
            None => return List::empty()
        };

        for &expression in expressions {
            builder.push(self.param_from_expression(expression));
        }

        builder.into_list()
    }

    #[inline]
    fn parameter_list(&mut self) -> ParameterList<'ast> {
        let mut builder = EmptyListBuilder::new(self.arena);
        let mut require_defaults = false;

        loop {
            let key = parameter_key!(self);
            let value = match self.lexer.token {
                OperatorAssign => {
                    self.lexer.consume();

                    require_defaults = true;

                    Some(self.expression(B1))
                },
                _ if require_defaults => return self.error(),
                _ => None
            };

            builder.push(self.alloc_in_loc(Parameter {
                key,
                value,
            }));

            match self.lexer.token {
                ParenClose => {
                    self.lexer.consume();

                    break;
                },
                Comma => {
                    self.lexer.consume();
                },
                _ => return self.error()
            }
        }

        builder.into_list()
    }
}

pub fn parse(source: &str) -> Result<Module, Vec<Error>> {
    let arena = Arena::new();

    let (body, errors) = {
        let mut parser = Parser::new(source, &arena);

        parser.parse();

        (parser.body.into_raw(), parser.errors)
    };

    match errors.len() {
        0 => Ok(Module::new(body, arena)),
        _ => Err(errors)
    }
}

#[cfg(test)]
mod mock {
    use super::*;
    use ast::{Expression, Literal, ExpressionPtr, StatementPtr, Block, BlockPtr, Name};
    use ast::statement::BlockStatement;

    pub struct Mock {
        arena: Arena
    }

    impl Mock {
        pub fn new() -> Self {
            Mock {
                arena: Arena::new()
            }
        }

        pub fn ptr<'a, T, I>(&'a self, val: I) -> Ptr<'a, Loc<T>> where
            T: 'a + Copy,
            I: Into<T>,
        {
            Ptr::new(self.arena.alloc(Loc::new(0, 0, val.into())))
        }

        pub fn name<'a, N>(&'a self, val: &'a str) -> N where
            N: Name<'a> + From<Ptr<'a, Loc<&'a str>>>,
        {
            N::from(Ptr::new(self.arena.alloc(Loc::new(0, 0, val))))
        }

        pub fn number<'a>(&'a self, number: &'static str) -> ExpressionPtr<'a> {
            self.ptr(Literal::Number(number))
        }

        pub fn block<'a, I, T, L>(&'a self, list: L) -> BlockPtr<'a, I> where
            I: Copy,
            T: Into<I> + Copy,
            L: AsRef<[T]>
        {
            self.ptr(Block { body: self.list(list) })
        }

        pub fn empty_block<'a, I: Copy>(&'a self) -> BlockPtr<'a, I> {
            self.ptr(Block { body: List::empty() })
        }

        pub fn list<'a, T, I, L>(&'a self, list: L) -> List<'a, Loc<T>> where
            T: 'a + Copy,
            L: AsRef<[I]>,
            I: Into<T> + Copy,
        {
            List::from_iter(&self.arena, list.as_ref().iter().cloned().map(|i| Loc::new(0, 0, i.into())))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use parser::mock::Mock;

    #[test]
    fn empty_parse() {
        let module = parse("").unwrap();

        assert_eq!(module.body(), List::empty());
    }

    #[test]
    fn empty_statements() {
        let module = parse(";;;").unwrap();
        let mock = Mock::new();

        let expected = mock.list([
            Statement::Empty,
            Statement::Empty,
            Statement::Empty
        ]);

        assert_eq!(module.body(), expected);
    }
}
