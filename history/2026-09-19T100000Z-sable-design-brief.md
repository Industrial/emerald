# Sable --- Language Design Specification and Design Brief

## 1. Purpose

This document describes a proposed new programming language,
provisionally called **Sable**.

Sable is intended to combine:

-   the simplicity, readability, and programmer ergonomics of **Ruby**;
-   the static typing, memory safety, performance, and concurrency
    guarantees associated with **Rust**;
-   the explicitness and correctness-oriented design of **Ada**;
-   algebraic data types and exhaustive pattern matching from ML-family
    languages;
-   modern functional-programming capabilities;
-   a deliberately small and coherent language surface.

The language is **not intended to be "Rust with Ruby syntax."** Its goal
is to reconsider language design from first principles while
incorporating lessons learned from existing languages.

The central design philosophy is:

> **Make programmer intent explicit, make ordinary code pleasant to
> read, and make the compiler enforce correctness without requiring
> unnecessary ceremony.**

A particularly important design decision is that Sable should be
**block-oriented**. Functions, methods, anonymous callable values,
conditionals, pattern matching, tests, and other executable constructs
should use a unified `do ... end` block model.

------------------------------------------------------------------------

# 2. Core Design Principles

Sable should follow these principles:

1.  **Static typing.**
2.  **No type inference.**
3.  **Types are explicitly declared.**
4.  **Mutability is explicitly declared.**
5.  **No implicit conversions.**
6.  **No `null`.**
7.  **`Option[T]` represents optional values.**
8.  **`Result[T, E]` represents expected recoverable failure.**
9.  **Unexpected failures may use exceptions/panics.**
10. **Algebraic data types are first-class.**
11. **Pattern matching is exhaustive.**
12. **Generics should be powerful and broadly Rust-like.**
13. **Traits/constraints should provide generic polymorphism.**
14. **Inheritance should not be the primary polymorphism mechanism.**
15. **Composition should be preferred to inheritance.**
16. **Functions are first-class values.**
17. **Blocks are the fundamental callable syntax.**
18. **There should be no separate lambda/block/Proc conceptual
    hierarchy.**
19. **`do ... end` should be used consistently throughout the
    language.**
20. **No user-defined macro system.**
21. **Documentation uses `#` comments, with `##` potentially reserved
    for documentation comments.**
22. **The standard toolchain should be cohesive and opinionated.**
23. **Memory safety and high performance are required goals.**
24. **Concurrency should be safe by construction.**
25. **The language should optimize for readability and maintainability
    rather than maximal feature count.**

------------------------------------------------------------------------

# 3. Relationship to Existing Languages

## Ruby

Sable should learn from Ruby's:

-   clean syntax;
-   lack of semicolons;
-   `do ... end` blocks;
-   expressive collections;
-   object-oriented ergonomics;
-   readable control flow;
-   REPL friendliness;
-   strong standard library;
-   general emphasis on programmer happiness.

However, Sable should avoid Ruby's:

-   dynamic typing;
-   runtime type errors;
-   implicitness where it harms readability;
-   extensive runtime metaprogramming;
-   ambiguity around blocks, lambdas, and Procs;
-   performance characteristics caused by its dynamic runtime model.

## Rust

Sable should learn from Rust's:

-   memory safety;
-   ownership concepts;
-   borrowing;
-   zero-cost abstractions;
-   deterministic resource management;
-   algebraic data types;
-   `Option`;
-   `Result`;
-   pattern matching;
-   traits;
-   generics;
-   fearless concurrency;
-   native compilation;
-   predictable performance.

However, Sable should reconsider how much of Rust's complexity must be
exposed directly to application programmers.

Rust's implementation and safety model should be an important
foundation, but Sable should strive for a simpler programmer-facing
model.

## Ada

Sable should learn from Ada's:

-   explicit declarations;
-   strong static typing;
-   domain-oriented types;
-   emphasis on correctness;
-   avoidance of implicit conversions;
-   ability to express programmer intent clearly.

## ML-family languages

Sable should learn from:

-   algebraic data types;
-   exhaustive pattern matching;
-   discriminated unions;
-   functional composition.

Unlike ML-family languages, however, Sable deliberately rejects
pervasive type inference.

------------------------------------------------------------------------

# 4. Explicit Typing

Sable does **not** infer variable types.

Every variable declaration must include a type.

Valid:

``` text
name: String = "Alice"
age: Int = 42
temperature: Float = 21.5
enabled: Bool = true
```

Mutable variables are explicit:

``` text
var count: Int = 0
```

Invalid:

``` text
name = "Alice"
age = 42
```

The purpose is not merely compiler correctness. Explicit types make code
easier to scan without following an expression back to its source.

The language should make it possible to understand a declaration
locally.

------------------------------------------------------------------------

# 5. Explicit Mutability

Bindings are immutable by default.

``` text
name: String = "Alice"
```

This binding cannot later be reassigned.

Mutable bindings use `var`:

``` text
var count: Int = 0

count = count + 1
```

This makes mutation visually apparent.

The compiler should reject:

``` text
name: String = "Alice"
name = "Bob"
```

This rule applies to local variables, fields, and other bindings
according to their declaration semantics.

------------------------------------------------------------------------

# 6. No Implicit Type Conversions

Sable should be strict about conversions.

For example, an `Int` should not silently become a `Float`.

Explicit conversion:

``` text
price: Float = Float(count)
```

Likewise:

``` text
value: Int = Int("123")
text: String = String(123)
```

Conversions that can fail should return `Result` or `Option` as
appropriate.

The principle is:

> A value should not silently change semantic type because the compiler
> guessed what the programmer intended.

------------------------------------------------------------------------

# 7. Object State Uses `@`

Sable adopts Ruby-style `@` instance-variable syntax.

Example:

``` text
class User
    id: Int
    name: String
    email: Email

    fn initialize(id: Int, name: String, email: Email) do
        @id = id
        @name = name
        @email = email
    end

    fn adult?(): Bool do
        @id >= 18
    end
end
```

`@name`, `@id`, etc. clearly indicate object instance state.

`@` should have a narrow, unambiguous meaning and should not be
overloaded with unrelated language features.

------------------------------------------------------------------------

# 8. Functions and the `do ... end` Model

A central Sable design decision is to use `do ... end` for function
bodies.

Instead of brace-based functions or an arrow-oriented syntax:

``` text
fn add(a: Int, b: Int): Int do
    a + b
end
```

The function declaration consists of:

-   `fn`;
-   a name;
-   typed parameters;
-   a colon;
-   an explicit return type;
-   `do`;
-   the body;
-   `end`.

Return types are mandatory.

No type inference is used for function return types.

------------------------------------------------------------------------

# 9. No Arrow Syntax

Sable should avoid `->` for function return types.

Use:

``` text
fn add(a: Int, b: Int): Int do
    a + b
end
```

not:

``` text
fn add(a: Int, b: Int) -> Int
```

Function types similarly use:

``` text
fn(Int, Int): Int
```

rather than:

``` text
fn(Int, Int) -> Int
```

Examples:

``` text
fn(): String
fn(Int): Bool
fn(String, Int): Result
```

This creates a consistent visual grammar.

------------------------------------------------------------------------

# 10. Functions Are First-Class Values

A named function should conceptually be a named binding of a function
value.

A named declaration:

``` text
fn add(a: Int, b: Int): Int do
    a + b
end
```

should be conceptually equivalent to:

``` text
add: fn(Int, Int): Int = do |a: Int, b: Int|
    a + b
end
```

The first form is the convenient named declaration.

The second form demonstrates the underlying model: functions are values.

This is important because Sable should not have a fundamental conceptual
distinction between:

-   named functions;
-   anonymous functions;
-   lambdas;
-   blocks.

Instead:

> **A function is a typed executable block.**

------------------------------------------------------------------------

# 11. Blocks Rather Than Lambdas

Sable should use **blocks** rather than introducing a separate lambda
construct.

Example:

``` text
double: fn(Int): Int = do |x: Int|
    x * 2
end
```

The `do ... end` construct creates the callable value.

There should not be a second syntax such as:

``` text
x => x * 2
```

or:

``` text
lambda(x) { ... }
```

unless a compelling future design reason emerges.

The goal is one unified abstraction.

------------------------------------------------------------------------

# 12. Block Parameters

Block parameters use pipe syntax, inspired by Ruby:

``` text
users.each do |user: User|
    print(user.name)
end
```

Multiple parameters:

``` text
items.each do |item: Item, index: Int|
    print(index)
    print(item)
end
```

Because Sable has no type inference, block parameter types are explicit.

------------------------------------------------------------------------

# 13. Higher-Order Functions

Because functions are first-class values, higher-order functions are
ordinary language constructs.

Example:

``` text
fn apply(
    operation: fn(Int): Int,
    value: Int
): Int do
    operation(value)
end
```

A function can be assigned to a variable:

``` text
double: fn(Int): Int = do |x: Int|
    x * 2
end

result: Int = apply(double, 21)
```

A block can also be supplied directly.

------------------------------------------------------------------------

# 14. Block-Passing Syntax

Sable should support Ruby-like trailing blocks for ergonomic
higher-order APIs.

Example:

``` text
result: Int = apply(21) do |x: Int|
    x * 2
end
```

This should be equivalent to passing the function value explicitly.

For collection operations:

``` text
users.map do |user: User|
    user.name
end
```

``` text
users.each do |user: User|
    print(user.name)
end
```

The trailing-block syntax should be syntactic sugar for passing a
function value.

------------------------------------------------------------------------

# 15. Closures

Blocks can capture surrounding values.

Example:

``` text
fn make_multiplier(value: Int): fn(Int): Int do
    do |number: Int|
        number * value
    end
end
```

Then:

``` text
double: fn(Int): Int = make_multiplier(2)
triple: fn(Int): Int = make_multiplier(3)
```

The inner block captures `value`.

The compiler/runtime should use an ownership model capable of
implementing closures safely and efficiently.

------------------------------------------------------------------------

# 16. Recursive Functions

Named functions can recursively refer to themselves.

``` text
fn factorial(n: Int): Int do
    if n <= 1 do
        1
    else
        n * factorial(n - 1)
    end
end
```

The language should define the binding semantics necessary for recursion
explicitly.

Function-valued variables should also have well-defined
recursive-binding semantics if recursive anonymous values are supported.

------------------------------------------------------------------------

# 17. Function Types

Function types have the form:

``` text
fn(ArgumentType1, ArgumentType2): ReturnType
```

Examples:

``` text
fn(): String
fn(Int): Bool
fn(String): Int
fn(Int, Int): Result[Int, Error]
```

A function type should itself be a normal static type.

For example:

``` text
formatter: fn(User): String = do |user: User|
    user.name
end
```

Functions can therefore be:

-   assigned;
-   stored;
-   passed as arguments;
-   returned from functions;
-   stored in collections;
-   stored in maps;
-   captured by closures.

------------------------------------------------------------------------

# 18. Methods

Methods use the same `do ... end` body syntax as functions.

``` text
class User
    name: String

    fn initialize(name: String) do
        @name = name
    end

    fn greet(prefix: String): String do
        prefix + ", " + @name
    end
end
```

There should be no separate method-body syntax.

The distinction between a function and method should be
semantic/name-resolution related, not a completely different body
construct.

------------------------------------------------------------------------

# 19. `Option`

Sable should have an explicit optional-value type.

Conceptually:

``` text
enum Option[T]
    Some(T)
    None
end
```

Example:

``` text
user: Option[User]
```

There is no `null`.

This prevents an entire class of null-reference errors.

Convenience operators can exist:

``` text
name: String = user?.name ?? "Unknown"
```

The exact semantics of `?.` and `??` should remain statically typed and
should not reintroduce implicit nullability.

------------------------------------------------------------------------

# 20. `Result`

Sable explicitly wants Rust-style `Result` semantics.

Conceptually:

``` text
enum Result[T, E]
    Ok(T)
    Err(E)
end
```

A function that can fail in an expected way declares that fact:

``` text
fn read_file(path: Path): Result[String, IoError] do
    ...
end
```

Expected errors are therefore part of the function's type.

------------------------------------------------------------------------

# 21. Error Propagation

The `?` operator should propagate errors.

Example:

``` text
fn load_config(path: Path): Result[Config, Error] do
    contents: String = read_file(path)?
    config: Config = parse_config(contents)?

    Ok(config)
end
```

The operator should work with `Result` and potentially other compatible
propagation types if the language later defines such a protocol.

The purpose is to retain explicit error semantics without requiring
verbose manual matching at every call site.

------------------------------------------------------------------------

# 22. Exceptions and Panics

Sable should not necessarily eliminate exceptions entirely.

Use `Result` for **expected, recoverable failure**.

Use exceptions/panics for **unexpected failure or violated invariants**.

The language should clearly distinguish the two categories.

Examples of expected failure:

-   file does not exist;
-   invalid user input;
-   database lookup fails;
-   network request fails.

Examples of unexpected failure:

-   violated internal invariant;
-   impossible program state;
-   unrecoverable runtime failure.

The exact exception model requires further design.

------------------------------------------------------------------------

# 23. Algebraic Data Types

Enums should support associated values.

Example:

``` text
enum Shape
    Circle(radius: Float)
    Rectangle(width: Float, height: Float)
    Point
end
```

This should provide a concise way to model domain states.

------------------------------------------------------------------------

# 24. Pattern Matching

Pattern matching should be a core language feature.

Example:

``` text
match shape do
    Circle(radius) do
        Math.pi * radius * radius
    end

    Rectangle(width, height) do
        width * height
    end

    Point do
        0.0
    end
end
```

Pattern matching must be statically checked for exhaustiveness.

The compiler should report missing cases.

Wildcard patterns should be supported:

``` text
match result do
    Ok(value) do
        print(value)
    end

    Err(error) do
        print(error)
    end
end
```

------------------------------------------------------------------------

# 25. Pattern Matching Uses Blocks

Pattern matching should use the same `do ... end` philosophy as the rest
of the language.

Example:

``` text
match result do
    Ok(value) do
        print(value)
    end

    Err(error) do
        print(error)
    end
end
```

This intentionally avoids introducing a separate arrow syntax such as:

``` text
Ok(value) -> ...
Err(error) -> ...
```

Every match arm is a block.

This makes `do ... end` a fundamental language abstraction rather than
merely a function-body convention.

------------------------------------------------------------------------

# 26. Conditional Constructs

Conditionals should use blocks.

``` text
if user.active do
    send_email(user)
end
```

With `else`:

``` text
if user.active do
    send_email(user)
else
    disable_user(user)
end
```

Nested conditions remain structurally explicit.

------------------------------------------------------------------------

# 27. Loops

Loops should also use blocks.

``` text
while connection.open? do
    process(connection.read())
end
```

Iteration:

``` text
for user in users do
    print(user.name)
end
```

The exact grammar of `for` should be finalized, but it should follow the
block model.

------------------------------------------------------------------------

# 28. Classes and Object Orientation

Sable should support classes because object-oriented modelling remains
useful.

Example:

``` text
class User
    id: Int
    name: String
    email: Email

    fn initialize(id: Int, name: String, email: Email) do
        @id = id
        @name = name
        @email = email
    end

    fn adult?(): Bool do
        @id >= 18
    end
end
```

However, traditional inheritance should not be the primary abstraction.

------------------------------------------------------------------------

# 29. No Class Inheritance as the Primary Polymorphism Mechanism

Avoid:

``` text
class Dog extends Animal
```

Instead use traits/interfaces and composition.

Example:

``` text
interface Animal
    fn speak(): String
end
```

``` text
class Dog
    fn speak(): String do
        "Woof"
    end
end
```

The language can then use static polymorphism through traits/interfaces.

The exact distinction between `interface`, `trait`, and other
abstractions remains to be designed.

------------------------------------------------------------------------

# 30. Rust-Like Generics

Sable should retain a powerful generic system rather than simplifying
generics excessively.

Example:

``` text
class Stack[T]
    items: List[T]

    fn push(value: T) do
        @items.push(value)
    end

    fn pop(): Option[T] do
        @items.pop()
    end
end
```

Generic constraints can resemble Rust:

``` text
fn maximum[T: Ord](values: List[T]): T do
    ...
end
```

Multiple constraints:

``` text
fn sort[T: Ord + Clone](values: List[T]): List[T] do
    ...
end
```

The language should preserve the expressive power needed for zero-cost
generic abstractions.

The exact trait system should be deliberately designed to avoid
unnecessary complexity where possible, but it should not sacrifice
essential generic expressiveness.

------------------------------------------------------------------------

# 31. Memory Management

Sable is intended to achieve memory safety and native-level performance.

Rust is the primary implementation/reference point for:

-   ownership;
-   borrowing;
-   deterministic destruction;
-   memory safety;
-   avoiding data races;
-   efficient value semantics.

However, Sable should investigate whether all of Rust's ownership and
lifetime syntax needs to be exposed to the programmer.

A desired principle is:

> **Programmers should only need to think explicitly about ownership
> when ownership materially affects the design or performance of the
> program.**

Potential syntax for explicit ownership concepts might look like:

``` text
fn process(data: borrow Data): Result do
    ...
end
```

or:

``` text
fn consume(data: own Data): Result do
    ...
end
```

These are conceptual examples, not finalized syntax.

The language should prioritize a simpler user model while retaining
Rust-level safety.

------------------------------------------------------------------------

# 32. Concurrency

Concurrency should be safe by construction.

Sable should learn from:

-   Rust's data-race prevention;
-   Erlang's message-passing model;
-   structured concurrency;
-   modern async runtimes.

A conceptual API might look like:

``` text
task download(url: Url): Data do
    ...
end
```

Parallel operations:

``` text
results: List[Data] = parallel urls.map(download)
```

Message passing:

``` text
channel: Channel[Message]
```

``` text
task worker(channel: Channel[Message]) do
    loop do
        message: Message = channel.receive()
        handle(message)
    end
end
```

Shared mutable state should be difficult to use accidentally.

The language should favor ownership transfer, immutability, and message
passing.

------------------------------------------------------------------------

# 33. Immutability

Immutability should be the default.

``` text
name: String = "Alice"
```

Mutation requires `var`.

This should extend conceptually into data structures and APIs where
practical.

Immutable data should be cheap to use, and persistent data structures
may be provided where appropriate.

------------------------------------------------------------------------

# 34. Metaprogramming

Sable should deliberately avoid Ruby-style runtime metaprogramming.

Avoid features such as:

-   arbitrary `method_missing`;
-   runtime class mutation;
-   arbitrary method definition;
-   class evaluation;
-   runtime syntax manipulation.

The reason is predictability, tooling quality, static analysis, and
compiler optimization.

The language should provide ordinary language mechanisms rather than
encouraging metaprogramming as a substitute for missing abstractions.

------------------------------------------------------------------------

# 35. No User Macros

Sable should have **no user-defined macro system**.

Do not add Rust-style procedural macros or a Ruby-like runtime
metaprogramming layer merely for extensibility.

If a feature requires compiler magic, it should ideally become a
language feature.

The guiding principle is:

> **There should be one obvious way to understand what code means.**

This also improves:

-   IDE support;
-   formatting;
-   static analysis;
-   compiler diagnostics;
-   documentation;
-   refactoring;
-   onboarding.

------------------------------------------------------------------------

# 36. Comments and Documentation

Comments should use `#`.

Example:

``` text
# Implementation comment.
```

Documentation comments can use `##`:

``` text
## Creates a new user.
##
## Returns UserError.AlreadyExists if the email already exists.
fn create_user(
    name: String,
    email: Email
): Result[User, UserError] do
    ...
end
```

The documentation tool should extract `##` comments.

No `//` syntax should be required.

The language should avoid multiple competing comment syntaxes unless
block comments are later demonstrated to be necessary.

------------------------------------------------------------------------

# 37. Testing

Testing should be a first-class language/toolchain feature.

Example:

``` text
test "calculates tax correctly" do
    price: Money = 100.00
    tax: Money = calculate_tax(price, 0.20)

    assert tax == 20.00
end
```

Tests should use ordinary `do ... end` blocks.

Pattern matching can be used for explicit result validation:

``` text
test "rejects invalid email" do
    result: Result[User, UserError] =
        create_user("Alice", "not-an-email")

    match result do
        Err(UserError.InvalidEmail) do
            # expected
        end

        _ do
            fail("Expected InvalidEmail")
        end
    end
end
```

Property testing should be supported:

``` text
property "sorting preserves elements" do
    ...
end
```

Benchmarking should also be integrated:

``` text
benchmark "parser" do
    parse(document)
end
```

The exact test API remains open.

------------------------------------------------------------------------

# 38. Collections

Sable should have expressive collection APIs inspired by Ruby.

Example:

``` text
users.map do |user: User|
    user.name
end
```

Filtering:

``` text
users.filter do |user: User|
    user.active
end
```

Iteration:

``` text
users.each do |user: User|
    print(user.name)
end
```

Sorting:

``` text
users.sort()
```

Chaining should be readable:

``` text
users
    .filter do |user: User|
        user.active
    end
    .map do |user: User|
        user.name
    end
    .sort()
```

The standard library should make common collection transformations
concise without requiring a large collection DSL.

------------------------------------------------------------------------

# 39. Domain Types and Units

Sable should make it easy to define domain-specific types.

Instead of:

``` text
distance: Float
speed: Float
```

a program can use:

``` text
distance: Meters
speed: MetersPerSecond
```

This prevents invalid operations at compile time.

Conceptually:

``` text
time: Seconds = distance / speed
```

The language/library should support lightweight value types so domain
modelling does not impose significant runtime cost.

Possible unit syntax:

``` text
100.m
20.mps
```

is illustrative and not finalized.

------------------------------------------------------------------------

# 40. Modules and Imports

Imports should be explicit.

``` text
import Http
import Json
import Database.Postgres
```

Exports should be explicit:

``` text
export User
export create_user
```

The module system should avoid magical visibility rules.

A cohesive package/dependency system should be part of the official
toolchain.

------------------------------------------------------------------------

# 41. Standard Toolchain

The language should ship with one official toolchain.

Conceptually:

``` text
sable build
sable run
sable test
sable format
sable lint
sable doc
sable repl
sable package
```

The formatter should be canonical.

The package manager should be canonical.

The language server should be canonical.

The testing system should be canonical.

The objective is to avoid fragmentation and ecosystem disputes over
basic development tooling.

------------------------------------------------------------------------

# 42. REPL

Ruby's REPL experience is valuable and should be retained.

Sable should provide:

``` text
sable repl
```

The REPL should understand the static type system.

Because Sable requires explicit types, REPL declarations should still be
typed:

``` text
name: String = "Alice"
age: Int = 42
```

The REPL should provide excellent compiler diagnostics and inspection
facilities.

------------------------------------------------------------------------

# 43. Example Complete Program

A representative Sable program:

``` text
import Http
import Json

## Represents a user account.
class User
    id: Int
    name: String
    email: Email

    fn initialize(
        id: Int,
        name: String,
        email: Email
    ) do
        @id = id
        @name = name
        @email = email
    end

    fn adult?(): Bool do
        @id >= 18
    end
end


enum UserError
    NotFound
    InvalidEmail
    AlreadyExists
end


fn find_user(id: Int): Result[User, UserError] do
    response: Response =
        Http.get("/users/" + String(id))?

    match response.status do
        200 do
            Json.decode[User](response.body)
        end

        404 do
            Err(UserError.NotFound)
        end

        _ do
            Err(UserError.NotFound)
        end
    end
end


fn main(): Int do
    result: Result[User, UserError] = find_user(42)

    match result do
        Ok(user) do
            print("Hello, " + user.name)
        end

        Err(UserError.NotFound) do
            print("User not found")
        end

        Err(error) do
            print("Error: " + String(error))
        end
    end

    0
end
```

This example illustrates:

-   explicit typing;
-   immutable bindings by default;
-   `var` for mutation;
-   `@` instance state;
-   classes;
-   methods;
-   functions;
-   `do ... end`;
-   `Result`;
-   pattern matching;
-   generic types;
-   error propagation;
-   documentation comments;
-   no arrows;
-   no braces;
-   no semicolons;
-   no type inference;
-   no `null`.

------------------------------------------------------------------------

# 44. Unified `do` Philosophy

The most important syntactic idea is that `do ... end` should not be
treated merely as Ruby-inspired syntax.

It should represent a **unified block abstraction**.

Examples:

## Function

``` text
fn add(a: Int, b: Int): Int do
    a + b
end
```

## Function value

``` text
add: fn(Int, Int): Int = do |a: Int, b: Int|
    a + b
end
```

## Method

``` text
fn greet(name: String): String do
    "Hello, " + name
end
```

## Collection block

``` text
users.each do |user: User|
    print(user.name)
end
```

## Conditional

``` text
if condition do
    action()
end
```

## Loop

``` text
while connection.open?() do
    process(connection.read())
end
```

## Pattern match

``` text
match result do
    Ok(value) do
        print(value)
    end

    Err(error) do
        print(error)
    end
end
```

## Test

``` text
test "works" do
    assert something()
end
```

The objective is syntactic and conceptual consistency.

------------------------------------------------------------------------

# 45. Potential Core Grammar

This is an illustrative grammar, not yet a finalized parser
specification.

``` text
function_decl =
    "fn" identifier "(" parameters? ")" ":" type block

function_value =
    "do" block_parameters? block_body "end"

block =
    "do" block_parameters? block_body "end"

block_parameters =
    "|" parameter_list? "|"

parameter =
    identifier ":" type

variable_decl =
    ("var"?) identifier ":" type "=" expression

class_decl =
    "class" identifier class_body "end"

enum_decl =
    "enum" identifier generic_parameters? enum_body "end"

if_expr =
    "if" expression "do"
        block_body
    ("else" "do"
        block_body)?
    "end"

match_expr =
    "match" expression "do"
        match_arm*
    "end"

match_arm =
    pattern "do" block_body "end"
```

This grammar should be refined through implementation rather than
treated as final.

------------------------------------------------------------------------

# 46. Things Explicitly Rejected

The current design explicitly rejects or questions:

-   dynamic typing;
-   implicit local-variable type inference;
-   implicit return-type inference;
-   implicit numeric conversions;
-   `null`;
-   mandatory brace syntax;
-   arrow-based return syntax;
-   separate lambda syntax;
-   separate Proc/block hierarchy;
-   primary reliance on class inheritance;
-   Ruby-style runtime metaprogramming;
-   user-defined macros;
-   Rust-style procedural macros;
-   unnecessary compiler magic;
-   semicolons;
-   `//` as the primary comment syntax.

------------------------------------------------------------------------

# 47. Important Open Design Questions

The following areas require further design work.

## Ownership syntax

How much of Rust's ownership model should be visible?

Possible principle:

-   ordinary code should feel automatic;
-   advanced code can explicitly state ownership/borrowing;
-   compiler must still guarantee memory safety.

## Trait versus interface

Should Sable have:

``` text
trait
```

and:

``` text
interface
```

as distinct concepts?

Or should there be a single trait-like abstraction?

## Classes versus structs

Should there be both:

``` text
class User
```

and:

``` text
struct User
```

with different semantics?

Potentially:

-   `struct` = value-oriented data;
-   `class` = identity/reference-oriented object.

This distinction needs careful consideration.

## Function/block semantics

Should every function value literally be represented by the same
internal type as a block?

The preferred conceptual model is yes.

## Method values

Should:

``` text
user.greet
```

produce a bound function value automatically?

If so, its type might be:

``` text
fn(String): String
```

depending on the method signature.

## Closures

How should closure captures interact with ownership and mutability?

## Async

Should `async`/`await` exist, or should structured concurrency and tasks
provide the primary abstraction?

## Exceptions

Exactly how should exceptions interact with static `Result` types?

## Type aliases and newtypes

How can Sable make domain types easy to create without introducing
unnecessary boilerplate?

## Operator overloading

Should operators be overloadable through traits?

## Numeric model

What integer widths and numeric conversions should exist?

## Generic variance

Should function and generic types follow Rust-like variance rules?

## Lifetime exposure

Can lifetime complexity be hidden in most application code without
weakening safety?

------------------------------------------------------------------------

# 48. Desired Programmer Experience

A programmer should be able to look at:

``` text
fn find_user(id: Int): Result[User, UserError] do
```

and immediately know:

-   the function is called `find_user`;
-   it accepts an `Int`;
-   it returns either a `User` or `UserError`;
-   the implementation is in the following `do ... end` block.

Likewise:

``` text
formatter: fn(User): String
```

should immediately tell the reader what the callable accepts and
returns.

And:

``` text
var count: Int
```

immediately tells the reader that the binding is mutable.

The language should optimize for **local comprehension**.

A programmer should not need to chase type inference through multiple
functions just to understand a variable.

------------------------------------------------------------------------

# 49. Overall Language Identity

Sable should feel like:

> **Ruby if Ruby had been designed from the beginning as a statically
> typed systems-capable language, with Rust's safety foundations and
> ML's data modelling, while refusing to expose unnecessary
> complexity.**

Its strongest distinguishing characteristics should be:

1.  **Explicit types everywhere.**
2.  **Ruby-like readability.**
3.  **`@` for instance state.**
4.  **`do ... end` as the universal executable-block construct.**
5.  **First-class typed functions built from blocks.**
6.  **Higher-order programming without a separate lambda syntax.**
7.  **Rust-like generics and traits.**
8.  **`Option` and `Result` as fundamental types.**
9.  **Exhaustive pattern matching.**
10. **Immutable-by-default bindings.**
11. **Rust-level memory safety and native performance as implementation
    goals.**
12. **No user macros.**
13. **No runtime metaprogramming.**
14. **No implicit conversions.**
15. **No type inference.**
16. **A cohesive official toolchain.**

The language should be powerful without feeling complicated.

------------------------------------------------------------------------

# 50. Design Maxim

The entire project can be summarized by the following rule:

> **Explicit where ambiguity matters; concise where intent is obvious;
> safe by default; and consistent everywhere.**

In particular:

> **`do ... end` is not merely syntax. It is the central abstraction
> around which Sable's executable model should be designed.**

Named functions are named blocks.

Anonymous functions are blocks assigned to values.

Higher-order functions accept blocks/function values.

Methods have block bodies.

Conditionals have blocks.

Loops have blocks.

Pattern-match arms have blocks.

Tests have blocks.

This gives Sable a single, coherent mental model for executable code
while preserving first-class functions, static typing, and powerful
generic programming.
