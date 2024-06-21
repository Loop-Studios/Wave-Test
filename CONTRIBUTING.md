<div align="center">
  <picture>
    <img alt="Wave Programming Language"
         src="https://wave-lang.dev/assets/img/features/wave.png"
         width="50%">
  </picture>

[🇺🇸][ENGLISH] [🇰🇷][KOREAN] [🇪🇸][SPANISH] [🇯🇵][JAPANESE]

</div>

[KOREAN]: .github/contributing/KOREAN.md
[ENGLISH]: CONTRIBUTING.md
[SPANISH]: .github/contributing/SPANISH.md
[JAPANESE]: .github/contributing/JAPANESE.md

<h1>⚠️Warning⚠️</h1>

**You must fully understand everything to contribute to this project without inconvenience.**

# How to Contribute to Wave

Wave is an open-source project. Anyone can contribute to the project.
However, if you want to contribute, there are a few things to keep in mind.

## Programming Languages

We use Rust as our main programming language.
However, once Wave is developed to a level where bootstrapping is possible, we plan to change the main programming language to Wave.

### Usable Programming Languages

* **[Wave](https://www.wave-lang.dev/)**
* **[Rust](https://www.rust-lang.org/)**
* C/C++
* Go
* Kotlin
* Carbon
* Haskell
* Lisp
* Dart
* ML
* Nim
* Python
* Mojo
* Zig

## Code Conventions

We are setting guidelines for code writing.

### K&R

If you use a coding style other than K&R (such as BSD, GNU, etc.), your pull request may be rejected.
Even if it passes, we may have to change it back to K&R, so please be aware.

#### Examples

* Correct example
```rust
fn main() {
    println!("Hello World!");
}
```

* Incorrect examples
```rust
fn main() 
{
    println!("Hello World!");
}
```

```rust
fn main() 
    {
        println!("Hello World!");
    }
```

## How to Contribute

### Fork

If you want to contribute to this project, we strongly recommend forking and working on it.

### Understanding the Project Structure

When contributing, it's important to first understand the project structure. Rust projects typically have the following structure:

```
project_root/
├── src/
│   ├── main.rs
│   ├── lib.rs
│   └── [feature-specific modules]
├── tests/
├── examples/
├── Cargo.toml
└── README.md
```

When adding new features or modifying existing code, you should write your code in the appropriate location for that feature. For example:

- When adding a new feature, create a new module within the `src/` directory or add functionality to an existing module.
- When fixing a bug, find the file containing the bug and modify it directly.
- When adding tests, create a new test file in the `tests/` directory or add tests to an existing test file.

Do not create folders with individual contributor names. Instead, track changes through Git and, if necessary, add contributor information to the CONTRIBUTORS file.


### Build

You should test that your code works properly before making a pull request.

### Pull Request

Pull requests for this project must be submitted to https://github.com/LunaStev/Wave.
When submitting a pull request, please clearly describe the following:

1. The purpose and functionality of your code
2. The programming language used
3. Libraries used (including any self-developed libraries)
    - For self-developed libraries, please provide a detailed explanation of their functions.
4. Frameworks used (including any self-developed frameworks)
5. Technologies or methodologies applied

By providing this information, you'll help project maintainers better understand and evaluate your contribution.