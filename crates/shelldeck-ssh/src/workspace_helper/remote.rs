//! Unix implementation used by the trusted OpenSSH subsystem helper.

use super::{
    decode_request, encode_response, invalid_data, RequestFrame, ResponseFrame,
    WorkspaceHelperErrorCode, WorkspaceHelperFailure, WorkspacePreparedReceipt, MAX_FRAME_BYTES,
    TOKEN_BYTES,
};
use rustix::fd::OwnedFd;
use rustix::fs::{fstat, open, openat, Mode, OFlags};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteHelperConfig {
    roots: Vec<PathBuf>,
    shell: PathBuf,
    git: PathBuf,
}

impl RemoteHelperConfig {
    pub fn from_args(arguments: impl IntoIterator<Item = String>) -> std::io::Result<Self> {
        let mut arguments = arguments.into_iter();
        let _program = arguments.next();
        let mut roots = Vec::new();
        let mut shell = PathBuf::from("/bin/sh");
        let mut git = PathBuf::from("/usr/bin/git");
        while let Some(argument) = arguments.next() {
            let value = arguments.next().ok_or_else(invalid_data)?;
            match argument.as_str() {
                "--root" => roots.push(validate_absolute_path(value)?),
                "--shell" => shell = validate_absolute_path(value)?,
                "--git" => git = validate_absolute_path(value)?,
                _ => return Err(invalid_data()),
            }
        }
        if roots.is_empty() || !shell.is_file() || !git.is_file() {
            return Err(invalid_data());
        }
        roots.sort();
        roots.dedup();
        Ok(Self { roots, shell, git })
    }

    #[cfg(test)]
    fn for_test(root: PathBuf, git: PathBuf) -> Self {
        Self {
            roots: vec![root],
            shell: PathBuf::from("/bin/sh"),
            git,
        }
    }
}

#[derive(Debug)]
struct RepositoryFacts {
    head_oid: String,
    branch: String,
}

#[derive(Debug)]
struct PreparedWorkspace {
    directory: OwnedFd,
    receipt: WorkspacePreparedReceipt,
    facts: RepositoryFacts,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RemoteExchange {
    Resume,
    Released,
    Refused,
}

/// Serve one prepare/resume exchange over stdin/stdout and replace the helper
/// with the configured shell only after the exact retained receipt is resumed.
pub fn run_stdio(config: RemoteHelperConfig) -> ExitCode {
    let exchange = {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut input = stdin.lock();
        let mut output = stdout.lock();
        exchange(&config, &mut input, &mut output)
    };
    match exchange {
        Ok(RemoteExchange::Resume) => {
            let error = Command::new(&config.shell)
                .arg("-l")
                .env_remove("SHELLDECK_WORKSPACE_ROOTS")
                .exec();
            let _ = error;
            ExitCode::FAILURE
        }
        Ok(RemoteExchange::Released) => ExitCode::SUCCESS,
        Ok(RemoteExchange::Refused) | Err(_) => ExitCode::FAILURE,
    }
}

pub fn exchange(
    config: &RemoteHelperConfig,
    input: &mut impl Read,
    output: &mut impl Write,
) -> std::io::Result<RemoteExchange> {
    let request = match read_request(input) {
        Ok(RequestFrame::Prepare(request)) => request,
        _ => {
            write_failure(output, WorkspaceHelperErrorCode::InvalidRequest, false)?;
            return Ok(RemoteExchange::Refused);
        }
    };
    let prepared = match prepare_workspace(config, request) {
        Ok(prepared) => prepared,
        Err(failure) => {
            write_response(output, &ResponseFrame::Error(failure))?;
            return Ok(RemoteExchange::Refused);
        }
    };
    write_response(output, &ResponseFrame::Prepared(prepared.receipt.clone()))?;

    match read_request(input) {
        Ok(RequestFrame::Release(token)) if token == prepared.receipt.token => {
            write_response(output, &ResponseFrame::Released(token))?;
            Ok(RemoteExchange::Released)
        }
        Ok(RequestFrame::Resume(token)) if token == prepared.receipt.token => {
            if let Err(failure) = revalidate_workspace(config, &prepared) {
                write_response(output, &ResponseFrame::Error(failure))?;
                return Ok(RemoteExchange::Refused);
            }
            rustix::process::fchdir(&prepared.directory).map_err(|_| invalid_data())?;
            let mut terminal = prepare_terminal_for_ready()?;
            write_response(output, &ResponseFrame::Ready(token))?;
            restore_terminal_output(&mut terminal)?;
            Ok(RemoteExchange::Resume)
        }
        _ => {
            write_failure(output, WorkspaceHelperErrorCode::StaleReceipt, false)?;
            Ok(RemoteExchange::Refused)
        }
    }
}

fn prepare_terminal_for_ready() -> std::io::Result<rustix::termios::Termios> {
    use rustix::termios::{
        tcgetattr, tcsetattr, ControlModes, InputModes, LocalModes, OptionalActions, OutputModes,
    };

    let stdin = std::io::stdin();
    let mut terminal = tcgetattr(&stdin).map_err(std::io::Error::from)?;
    terminal
        .input_modes
        .insert(InputModes::BRKINT | InputModes::ICRNL | InputModes::IXON);
    terminal.control_modes.remove(ControlModes::CSIZE);
    terminal
        .control_modes
        .insert(ControlModes::CS8 | ControlModes::CREAD);
    terminal.local_modes.insert(
        LocalModes::ISIG
            | LocalModes::ICANON
            | LocalModes::ECHO
            | LocalModes::ECHOE
            | LocalModes::ECHOK
            | LocalModes::IEXTEN,
    );
    // Keep output processing disabled until the final binary Ready frame has
    // been flushed, so a random token byte can never be rewritten as CRLF.
    terminal.output_modes.remove(OutputModes::OPOST);
    tcsetattr(&stdin, OptionalActions::Now, &terminal).map_err(std::io::Error::from)?;
    Ok(terminal)
}

fn restore_terminal_output(terminal: &mut rustix::termios::Termios) -> std::io::Result<()> {
    use rustix::termios::{tcsetattr, OptionalActions, OutputModes};

    terminal
        .output_modes
        .insert(OutputModes::OPOST | OutputModes::ONLCR);
    tcsetattr(std::io::stdin(), OptionalActions::Now, terminal).map_err(std::io::Error::from)
}

fn read_request(input: &mut impl Read) -> std::io::Result<RequestFrame> {
    let mut prefix = [0_u8; 4];
    input.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(invalid_data());
    }
    let mut payload = vec![0_u8; length];
    input.read_exact(&mut payload)?;
    decode_request(&payload)
}

fn write_response(output: &mut impl Write, response: &ResponseFrame) -> std::io::Result<()> {
    output.write_all(&encode_response(response)?)?;
    output.flush()
}

fn write_failure(
    output: &mut impl Write,
    code: WorkspaceHelperErrorCode,
    retryable: bool,
) -> std::io::Result<()> {
    write_response(
        output,
        &ResponseFrame::Error(WorkspaceHelperFailure { code, retryable }),
    )
}

fn prepare_workspace(
    config: &RemoteHelperConfig,
    request: super::WorkspacePrepareRequest,
) -> std::result::Result<PreparedWorkspace, WorkspaceHelperFailure> {
    let remote_root = validate_remote_root(&request.remote_root)
        .map_err(|_| failure(WorkspaceHelperErrorCode::UnauthorizedRoot, false))?;
    let authorized_root = config
        .roots
        .iter()
        .filter(|root| remote_root.starts_with(root))
        .max_by_key(|root| root.components().count())
        .ok_or_else(|| failure(WorkspaceHelperErrorCode::UnauthorizedRoot, false))?;
    let mut directory = open_absolute_directory(authorized_root)
        .map_err(|_| failure(WorkspaceHelperErrorCode::WorkspaceUnavailable, true))?;
    let relative = remote_root
        .strip_prefix(authorized_root)
        .map_err(|_| failure(WorkspaceHelperErrorCode::UnauthorizedRoot, false))?;
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(failure(WorkspaceHelperErrorCode::UnauthorizedRoot, false));
        };
        directory = openat(&directory, component, directory_flags(), Mode::empty())
            .map_err(|_| failure(WorkspaceHelperErrorCode::WorkspaceUnavailable, true))?;
    }
    rustix::process::fchdir(&directory)
        .map_err(|_| failure(WorkspaceHelperErrorCode::WorkspaceUnavailable, true))?;
    let facts = repository_facts(config)?;
    let stat = fstat(&directory)
        .map_err(|_| failure(WorkspaceHelperErrorCode::WorkspaceUnavailable, true))?;
    let mut token = [0_u8; TOKEN_BYTES];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut token))
        .map_err(|_| failure(WorkspaceHelperErrorCode::Internal, true))?;
    Ok(PreparedWorkspace {
        directory,
        receipt: WorkspacePreparedReceipt {
            token,
            operation: request.operation,
            workspace: request.workspace,
            directory_device: stat.st_dev as u64,
            directory_inode: stat.st_ino as u64,
            head_oid: facts.head_oid.clone(),
            branch: facts.branch.clone(),
        },
        facts,
    })
}

fn revalidate_workspace(
    config: &RemoteHelperConfig,
    prepared: &PreparedWorkspace,
) -> std::result::Result<(), WorkspaceHelperFailure> {
    let stat = fstat(&prepared.directory)
        .map_err(|_| failure(WorkspaceHelperErrorCode::WorkspaceUnavailable, true))?;
    if stat.st_dev as u64 != prepared.receipt.directory_device
        || stat.st_ino as u64 != prepared.receipt.directory_inode
    {
        return Err(failure(WorkspaceHelperErrorCode::StaleReceipt, false));
    }
    rustix::process::fchdir(&prepared.directory)
        .map_err(|_| failure(WorkspaceHelperErrorCode::WorkspaceUnavailable, true))?;
    let facts = repository_facts(config)?;
    if facts.head_oid != prepared.facts.head_oid || facts.branch != prepared.facts.branch {
        return Err(failure(WorkspaceHelperErrorCode::StaleReceipt, false));
    }
    Ok(())
}

fn repository_facts(
    config: &RemoteHelperConfig,
) -> std::result::Result<RepositoryFacts, WorkspaceHelperFailure> {
    let inside = git_scalar(config, &["rev-parse", "--is-inside-work-tree"], 16)?;
    let prefix = git_scalar(config, &["rev-parse", "--show-prefix"], 4096)?;
    if inside != "true" || !prefix.is_empty() {
        return Err(failure(
            WorkspaceHelperErrorCode::RepositoryUnavailable,
            false,
        ));
    }
    let head_oid = git_scalar(config, &["rev-parse", "--verify", "HEAD"], 128)?;
    if !matches!(head_oid.len(), 40 | 64) || !head_oid.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(failure(
            WorkspaceHelperErrorCode::RepositoryUnavailable,
            false,
        ));
    }
    let branch = git_scalar(config, &["symbolic-ref", "--quiet", "--short", "HEAD"], 255)?;
    if branch.is_empty() {
        return Err(failure(
            WorkspaceHelperErrorCode::RepositoryUnavailable,
            false,
        ));
    }
    if !git_quiet(config, &["diff-index", "--quiet", "HEAD", "--"])?
        || !git_quiet(config, &["diff-files", "--quiet", "--"])?
        || git_has_output(
            config,
            &[
                "ls-files",
                "--others",
                "--exclude-standard",
                "--directory",
                "-z",
            ],
        )?
    {
        return Err(failure(WorkspaceHelperErrorCode::DirtyWorkspace, false));
    }
    Ok(RepositoryFacts { head_oid, branch })
}

fn git_command(config: &RemoteHelperConfig, arguments: &[&str]) -> Command {
    let mut command = Command::new(&config.git);
    command
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_0", "/dev/null")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn git_scalar(
    config: &RemoteHelperConfig,
    arguments: &[&str],
    limit: usize,
) -> std::result::Result<String, WorkspaceHelperFailure> {
    let mut child = git_command(config, arguments)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| failure(WorkspaceHelperErrorCode::RepositoryUnavailable, true))?;
    let mut output = Vec::new();
    child
        .stdout
        .take()
        .expect("piped git stdout")
        .take((limit + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|_| failure(WorkspaceHelperErrorCode::RepositoryUnavailable, true))?;
    let status = child
        .wait()
        .map_err(|_| failure(WorkspaceHelperErrorCode::RepositoryUnavailable, true))?;
    if !status.success() || output.len() > limit || output.contains(&0) {
        return Err(failure(
            WorkspaceHelperErrorCode::RepositoryUnavailable,
            false,
        ));
    }
    let output = std::str::from_utf8(&output)
        .map_err(|_| failure(WorkspaceHelperErrorCode::RepositoryUnavailable, false))?
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    Ok(output)
}

fn git_quiet(
    config: &RemoteHelperConfig,
    arguments: &[&str],
) -> std::result::Result<bool, WorkspaceHelperFailure> {
    git_command(config, arguments)
        .stdout(Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|_| failure(WorkspaceHelperErrorCode::RepositoryUnavailable, true))
}

fn git_has_output(
    config: &RemoteHelperConfig,
    arguments: &[&str],
) -> std::result::Result<bool, WorkspaceHelperFailure> {
    let mut child = git_command(config, arguments)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| failure(WorkspaceHelperErrorCode::RepositoryUnavailable, true))?;
    let mut byte = [0_u8; 1];
    let count = child
        .stdout
        .take()
        .expect("piped git stdout")
        .read(&mut byte)
        .map_err(|_| failure(WorkspaceHelperErrorCode::RepositoryUnavailable, true))?;
    if count != 0 {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|_| failure(WorkspaceHelperErrorCode::RepositoryUnavailable, true))?;
    if count == 0 && !status.success() {
        return Err(failure(
            WorkspaceHelperErrorCode::RepositoryUnavailable,
            false,
        ));
    }
    Ok(count != 0)
}

fn validate_remote_root(value: &str) -> std::io::Result<PathBuf> {
    if value.len() > 4096
        || !value.starts_with('/')
        || value.contains('\0')
        || value.contains("//")
        || value.ends_with('/')
        || value
            .split('/')
            .skip(1)
            .any(|part| part == "." || part == "..")
    {
        return Err(invalid_data());
    }
    validate_absolute_path(value.to_owned())
}

fn validate_absolute_path(value: String) -> std::io::Result<PathBuf> {
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(invalid_data());
    }
    Ok(path)
}

fn open_absolute_directory(path: &Path) -> std::io::Result<OwnedFd> {
    let mut directory =
        open("/", directory_flags(), Mode::empty()).map_err(std::io::Error::from)?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(component) => {
                directory = openat(&directory, component, directory_flags(), Mode::empty())
                    .map_err(std::io::Error::from)?;
            }
            _ => return Err(invalid_data()),
        }
    }
    Ok(directory)
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW
}

fn failure(code: WorkspaceHelperErrorCode, retryable: bool) -> WorkspaceHelperFailure {
    WorkspaceHelperFailure { code, retryable }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;

    struct TestRepository {
        root: PathBuf,
        git: PathBuf,
    }

    impl TestRepository {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "shelldeck-workspace-helper-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&root).unwrap();
            let git = PathBuf::from("/usr/bin/git");
            for arguments in [
                vec!["init", "-b", "main"],
                vec!["config", "user.name", "ShellDeck Test"],
                vec!["config", "user.email", "test@example.invalid"],
            ] {
                assert!(Command::new(&git)
                    .args(arguments)
                    .current_dir(&root)
                    .status()
                    .unwrap()
                    .success());
            }
            fs::write(root.join("tracked.txt"), "clean\n").unwrap();
            assert!(Command::new(&git)
                .args(["add", "tracked.txt"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success());
            assert!(Command::new(&git)
                .args(["commit", "-m", "initial"])
                .current_dir(&root)
                .stdout(Stdio::null())
                .status()
                .unwrap()
                .success());
            Self { root, git }
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn retained_descriptor_receipt_revalidates_clean_exact_lineage() {
        let repository = TestRepository::new();
        let config = RemoteHelperConfig::for_test(repository.root.clone(), repository.git.clone());
        let prepared = prepare_workspace(
            &config,
            super::super::WorkspacePrepareRequest {
                operation: uuid::Uuid::from_u128(1),
                workspace: uuid::Uuid::from_u128(2),
                remote_root: repository.root.to_string_lossy().into_owned(),
            },
        )
        .unwrap();
        assert_eq!(prepared.receipt.branch, "main");
        assert!(matches!(prepared.receipt.head_oid.len(), 40 | 64));
        revalidate_workspace(&config, &prepared).unwrap();

        fs::write(repository.root.join("untracked.txt"), "new\n").unwrap();
        assert_eq!(
            revalidate_workspace(&config, &prepared).unwrap_err().code,
            WorkspaceHelperErrorCode::DirtyWorkspace
        );
    }

    #[test]
    fn root_walk_refuses_symlink_components_and_outside_paths() {
        use std::os::unix::fs::symlink;

        let repository = TestRepository::new();
        let parent = repository.root.parent().unwrap().to_path_buf();
        let link = parent.join(format!("shelldeck-helper-link-{}", uuid::Uuid::new_v4()));
        symlink(&repository.root, &link).unwrap();
        let config = RemoteHelperConfig::for_test(parent.clone(), repository.git.clone());
        let error = prepare_workspace(
            &config,
            super::super::WorkspacePrepareRequest {
                operation: uuid::Uuid::from_u128(3),
                workspace: uuid::Uuid::from_u128(4),
                remote_root: link.to_string_lossy().into_owned(),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, WorkspaceHelperErrorCode::WorkspaceUnavailable);
        fs::remove_file(link).unwrap();

        let config = RemoteHelperConfig::for_test(repository.root.clone(), repository.git.clone());
        let outside = parent.join("outside");
        let error = prepare_workspace(
            &config,
            super::super::WorkspacePrepareRequest {
                operation: uuid::Uuid::from_u128(5),
                workspace: uuid::Uuid::from_u128(6),
                remote_root: outside.to_string_lossy().into_owned(),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, WorkspaceHelperErrorCode::UnauthorizedRoot);
    }

    #[test]
    fn exchange_refuses_wrong_receipt_without_starting_shell() {
        let repository = TestRepository::new();
        let config = RemoteHelperConfig::for_test(repository.root.clone(), repository.git.clone());
        let prepare = super::super::encode_request(&RequestFrame::Prepare(
            super::super::WorkspacePrepareRequest {
                operation: uuid::Uuid::from_u128(7),
                workspace: uuid::Uuid::from_u128(8),
                remote_root: repository.root.to_string_lossy().into_owned(),
            },
        ))
        .unwrap();
        let release =
            super::super::encode_request(&RequestFrame::Release([0; TOKEN_BYTES])).unwrap();
        let mut input = Cursor::new([prepare, release].concat());
        let mut output = Vec::new();
        assert_eq!(
            exchange(&config, &mut input, &mut output).unwrap(),
            RemoteExchange::Refused
        );
        let first_length = u32::from_be_bytes(output[..4].try_into().unwrap()) as usize;
        let second = &output[first_length + 4..];
        let second_length = u32::from_be_bytes(second[..4].try_into().unwrap()) as usize;
        assert_eq!(
            super::super::decode_response(&second[4..second_length + 4]).unwrap(),
            ResponseFrame::Error(WorkspaceHelperFailure {
                code: WorkspaceHelperErrorCode::StaleReceipt,
                retryable: false,
            })
        );
    }

    #[test]
    fn cli_requires_at_least_one_fixed_absolute_root() {
        assert!(RemoteHelperConfig::from_args(["helper".into()]).is_err());
        assert!(RemoteHelperConfig::from_args([
            "helper".into(),
            "--root".into(),
            "relative".into(),
        ])
        .is_err());
    }

    #[test]
    fn absolute_path_parser_rejects_parent_components() {
        assert!(validate_remote_root("/srv/workspaces/../secret").is_err());
        assert!(validate_remote_root("/srv//workspace").is_err());
        assert!(validate_remote_root("/srv/workspace/").is_err());
        assert_eq!(
            validate_remote_root("/srv/workspace").unwrap(),
            Path::new("/srv/workspace")
        );
    }
}
