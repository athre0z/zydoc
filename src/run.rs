use crate::prelude::*;

use std::io::Write as _;

/// Documentation generator for Zydis.
#[derive(Debug, argh::FromArgs)]
struct Args {
    /// path to the git repository
    #[argh(option)]
    repo: path::PathBuf,
    /// git references to generate documentation for
    #[argh(option)]
    refs: Vec<String>,
    /// output directory
    #[argh(option)]
    output_dir: path::PathBuf,
}

/// Run the actual application.
pub fn run() -> Result<()> {
    // Parse command-line.
    let args: Args = argh::from_env();

    // Create output directory.
    ensure!(!args.output_dir.exists(), "output directory already exists");
    fs::create_dir(&args.output_dir).context("failed to create directory")?;

    // Create absolute output directory path.
    let output_dir = args
        .output_dir
        .canonicalize()
        .context("failed to normalize path")?;

    // Checkout master..
    let repo = crate::git::Repo::new(&args.repo);
    repo.checkout("master")
        .context("failed to switch to master")?;

    // Read config from master.
    let config = fs::read_to_string(args.repo.join("Doxyfile"));
    let config = config.context("failed to read Doxyfile")?;

    // Parse regular expressions.
    let regexps = args
        .refs
        .iter()
        .map(|x| regex::Regex::new(&x).map_err(Into::into))
        .collect::<Result<Vec<_>>>()
        .context("failed to parse regular expression")?;

    let mut index = IndexContext::default();
    for git_ref in repo.refs()? {
        if !regexps.iter().any(|re| re.is_match(&git_ref)) {
            continue;
        }

        println!("Generating documentation for reference `{}`", &git_ref);

        // Create the output directory for this ref.
        let short_ref = short_ref_name(&git_ref);
        let slug = short_ref.replace('/', "-");
        let dir = output_dir.join(&slug);
        fs::create_dir(&dir).context("failed to create dir for ref")?;

        // Checkout ref.
        repo.checkout(&git_ref)?;

        // Run doxygen.
        //
        // Doxygen doesn't support overriding configurations via command-line switch,
        // so in order to customize the output directory, we make it read the config
        // from stdin, generating a custom configuration for each invocation.
        let mut proc = Command::new("doxygen")
            .current_dir(&args.repo)
            .arg("-")
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::piped())
            .stderr(process::Stdio::piped())
            .spawn()
            .context("failed to run doxygen")?;

        let local_config = format!("{}\nOUTPUT_DIRECTORY = {}", config, &dir.to_string_lossy());

        proc.stdin
            .as_ref()
            .expect("process created with piped stdin")
            .write_all(local_config.as_bytes())
            .context("failed to write doxygen config to stdin")?;

        let status = proc.wait().context("failed to wait for doxygen")?;
        if !status.success() {
            bail!("doxygen failed with status {}", status);
        }

        // Categorize and add to index.
        let ref_vec = if git_ref.starts_with("refs/tags") {
            &mut index.tags
        } else if git_ref.starts_with("refs/heads") {
            &mut index.branches
        } else {
            &mut index.misc_refs
        };

        ref_vec.push(IndexRef {
            git_ref: git_ref.clone(),
            short_ref: short_ref.to_owned(),
            dir: dir.as_path().to_string_lossy().into_owned(),
        });
    }

    repo.checkout("master")?;

    // Generate `index.html`.
    println!("Writing index.html");
    let index = render_index(index).context("failed to generate index.html")?;
    fs::write(output_dir.join("index.html"), index).context("failed to write index.html")?;

    Ok(())
}

#[derive(Debug, serde::Serialize)]
struct IndexRef {
    short_ref: String,
    git_ref: String,
    dir: String,
}

#[derive(Debug, Default, serde::Serialize)]
struct IndexContext {
    tags: Vec<IndexRef>,
    branches: Vec<IndexRef>,
    misc_refs: Vec<IndexRef>,
}

fn render_index(index: IndexContext) -> Result<String> {
    let mut hb = handlebars::Handlebars::new();
    hb.register_template_string("index", &include_str!("index.hbs"))
        .context("failed to register index template")?;
    hb.render("index", &index).map_err(Into::into)
}

fn short_ref_name(x: &str) -> &str {
    let x = x.strip_prefix("refs/").unwrap_or(x);
    let x = x.strip_prefix("heads/").unwrap_or(x);
    let x = x.strip_prefix("tags/").unwrap_or(x);
    x
}
