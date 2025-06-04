mod ids;
pub(crate) mod tool_metrics;

use ::fs::RealFs;
use clap::Parser;
use client::{Client, ProxySettings, UserStore};
use gpui::http_client::read_proxy_from_env;
use gpui::{App, AppContext, Application, AsyncApp, Entity, SemanticVersion};
use gpui_tokio::Tokio;
use language::LanguageRegistry;
use language_model::{ConfiguredModel, LanguageModelProviderId, LanguageModelRegistry};
use language_models;
use node_runtime::{NodeBinaryOptions, NodeRuntime};
use project::Project;
use project::project_settings::ProjectSettings;
use prompt_store::PromptBuilder;
use release_channel::AppVersion;
use reqwest_client::ReqwestClient;
use settings::{Settings, SettingsStore};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "zed-agent", disable_version_flag = true)]
struct Args {
    /// Path to the codebase (default: current directory)
    #[arg(long, short = 'p', default_value = ".")]
    path: String,
    
    /// Model provider to use
    #[arg(long, value_enum, default_value = "anthropic")]
    provider: Provider,
    
    /// Model name to use (if not specified, uses default for provider)
    #[arg(long, short = 'm')]
    model: Option<String>,
    
    /// Enable debug logging with detailed output
    #[arg(long)]
    debug: bool,
    
    /// Enable verbose logging (INFO level)
    #[arg(long)]
    verbose: bool,
    
    /// Suppress most output (WARNING+ only)
    #[arg(long)]
    quiet: bool,
    
    /// Disable colored output
    #[arg(long)]
    no_color: bool,
    
    /// Write logs to specified file
    #[arg(long)]
    log_file: Option<String>,
    
    /// Use the terminal CLI (default mode)
    #[arg(long)]
    cli: bool,
    
    /// Start the REST API server
    #[arg(long)]
    api: bool,
    
    /// Test mode: Initialize CLI, create agent, run basic tests, and exit with maximum logging
    #[arg(long)]
    test: bool,
    
    /// Type of agent to use
    #[arg(long, short = 'a', value_enum, default_value = "chat")]
    agent: AgentType,
    
    /// Run a single command and exit (non-interactive mode)
    #[arg(long, short = 'c')]
    command: Option<String>,
    
    /// Use interactive system message for more conversational experience
    #[arg(long, default_value = "true")]
    interactive: bool,
    
    /// Use standard system message instead of interactive mode
    #[arg(long)]
    standard: bool,
    
    /// API server host
    #[arg(long, default_value = "0.0.0.0")]
    api_host: String,
    
    /// API server port
    #[arg(long, default_value = "8000")]
    api_port: u16,
    
    /// Run task agent in headless mode with the given task description
    #[arg(long, short = 't')]
    task: Option<String>,
    
    /// Run task in current terminal instead of creating a new one
    #[arg(long)]
    no_new_terminal: bool,
    
    /// Reset GitHub authentication by clearing cached tokens
    #[arg(long)]
    reset_github_auth: bool,
    
    /// Clear GitHub authentication tokens without performing new login
    #[arg(long)]
    clear_github_auth: bool,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum Provider {
    Vertex,
    VertexClaude,
    Google,
    Copilot,
    Openai,
    Anthropic,
    Xai,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum AgentType {
    Chat,
    Code,
}

fn main() {
    dotenv::from_filename(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".env"),
    )
    .ok();

    env_logger::init();

    let system_id = ids::get_or_create_id(&ids::eval_system_id_path()).ok();
    let installation_id = ids::get_or_create_id(&ids::eval_installation_id_path()).ok();
    let session_id = uuid::Uuid::new_v4().to_string();

    let args = Args::parse();
    
    // Handle authentication clearing/reset first
    if args.reset_github_auth || args.clear_github_auth {
        println!("GitHub auth management not yet implemented");
        return;
    }
    
    // Handle special modes
    if args.test {
        println!("Running in test mode with maximum logging...");
    }
    
    if args.api {
        println!("Starting REST API server on {}:{}", args.api_host, args.api_port);
        // TODO: Implement API server
        return;
    }
    
    // Determine model based on provider and explicit model arg
    let model_name = determine_model(&args.provider, &args.model);
    
    // Handle single command execution
    if let Some(command) = &args.command {
        println!("Executing command: {}", command);
        // TODO: Implement single command execution
        return;
    }
    
    // Handle task mode
    if let Some(task) = &args.task {
        println!("Running task agent: {}", task);
        // TODO: Implement task agent
        return;
    }
    
    println!("Starting {} agent in {} mode...", 
             format!("{:?}", args.agent).to_lowercase(),
             if args.cli { "CLI" } else { "default" });

    let http_client = Arc::new(ReqwestClient::new());
    let app = Application::headless().with_http_client(http_client.clone());

    app.run(move |cx| {
        let app_state = init(cx);

        let telemetry = app_state.client.telemetry();
        telemetry.start(system_id, installation_id, session_id, cx);

        // Try to load model, but handle gracefully if providers aren't available
        match load_model(&model_name, cx) {
            Ok(agent_model) => {
                LanguageModelRegistry::global(cx).update(cx, |registry, cx| {
                    registry.set_default_model(Some(agent_model.clone()), cx);
                });

                let auth = agent_model.provider.authenticate(cx);

                cx.spawn(async move |cx| {
                    if let Err(e) = auth.await {
                        println!("Warning: Authentication failed: {}. Running in offline mode.", e);
                    }

                    // Load project context from specified path
                    let project_path = std::path::Path::new(&args.path);
                    println!("Loading project from: {}", project_path.display());
                    
                    start_agent_session(&args, cx.clone()).await?;
                    
                    anyhow::Ok(())
                })
                .detach();
            }
            Err(e) => {
                println!("Warning: Could not load language model: {}. Running in offline mode.", e);
                
                cx.spawn(async move |cx| {
                    // Load project context from specified path
                    let project_path = std::path::Path::new(&args.path);
                    println!("Loading project from: {}", project_path.display());
                    
                    start_agent_session(&args, cx.clone()).await?;
                    
                    anyhow::Ok(())
                })
                .detach();
            }
        }
    });
}

async fn start_agent_session(args: &Args, _cx: AsyncApp) -> anyhow::Result<()> {
    use std::io::{self, Write};
    
    println!("Zed Agent - Headless Mode");
    println!("Type 'exit' to quit, 'help' for commands");
    println!("Working directory: {}", args.path);
    println!();
    
    loop {
        print!("> ");
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();
        
        if input.is_empty() {
            continue;
        }
        
        match input {
            "exit" | "quit" => {
                println!("Goodbye!");
                break;
            }
            "help" => {
                println!("Commands:");
                println!("  help     - Show this help");
                println!("  exit     - Exit the agent");
                println!("  status   - Show agent status");
                println!("  context  - Show current context");
                println!("  Or just type a message to chat with the agent");
            }
            "status" => {
                println!("Agent Status:");
                println!("  Provider: {:?}", args.provider);
                println!("  Agent Type: {:?}", args.agent);
                println!("  Path: {}", args.path);
            }
            "context" => {
                println!("Current Context:");
                println!("  Working directory: {}", args.path);
                // TODO: Show loaded files and context
            }
            _ => {
                println!("Agent response: I received your message: '{}'", input);
                // TODO: Send to actual language model and get response
            }
        }
    }
    
    Ok(())
}

fn determine_model(provider: &Provider, explicit_model: &Option<String>) -> String {
    if let Some(model) = explicit_model {
        return model.clone();
    }
    
    match provider {
        Provider::Anthropic => "anthropic/claude-3-7-sonnet-latest".to_string(),
        Provider::Openai => "openai/gpt-4".to_string(),
        Provider::Google => "google/gemini-pro".to_string(),
        Provider::Vertex => "vertex/gemini-pro".to_string(),
        Provider::VertexClaude => "vertex/claude-3-sonnet".to_string(),
        Provider::Copilot => "copilot/gpt-4".to_string(),
        Provider::Xai => "xai/grok-beta".to_string(),
    }
}

/// Subset of `workspace::AppState` needed by `HeadlessAssistant`, with additional fields.
pub struct AgentAppState {
    pub languages: Arc<LanguageRegistry>,
    pub client: Arc<Client>,
    pub user_store: Entity<UserStore>,
    pub fs: Arc<dyn fs::Fs>,
    pub node_runtime: NodeRuntime,
    pub prompt_builder: Arc<PromptBuilder>,
}

pub fn init(cx: &mut App) -> Arc<AgentAppState> {
    release_channel::init(SemanticVersion::default(), cx);
    gpui_tokio::init(cx);

    let mut settings_store = SettingsStore::new(cx);
    settings_store
        .set_default_settings(settings::default_settings().as_ref(), cx)
        .unwrap();
    cx.set_global(settings_store);
    client::init_settings(cx);

    let user_agent = format!(
        "Zed/{} ({}; {})",
        AppVersion::global(cx),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let proxy_str = ProxySettings::get_global(cx).proxy.to_owned();
    let proxy_url = proxy_str
        .as_ref()
        .and_then(|input| input.parse().ok())
        .or_else(read_proxy_from_env);
    let http = {
        let _guard = Tokio::handle(cx).enter();
        ReqwestClient::proxy_and_user_agent(proxy_url, &user_agent)
            .expect("could not start HTTP client")
    };
    cx.set_http_client(Arc::new(http));

    Project::init_settings(cx);
    let client = Client::production(cx);
    cx.set_http_client(client.http_client());

    let git_binary_path = None;
    let fs = Arc::new(RealFs::new(
        git_binary_path,
        cx.background_executor().clone(),
    ));

    let mut languages = LanguageRegistry::new(cx.background_executor().clone());
    languages.set_language_server_download_dir(paths::languages_dir().clone());
    let languages = Arc::new(languages);

    let user_store = cx.new(|cx| UserStore::new(client.clone(), cx));

    extension::init(cx);
    language_model::init(client.clone(), cx);
    language_models::init(user_store.clone(), client.clone(), fs.clone(), cx);

    let (tx, rx) = async_watch::channel(None);
    cx.observe_global::<SettingsStore>(move |cx| {
        let settings = &ProjectSettings::get_global(cx).node;
        let options = NodeBinaryOptions {
            allow_path_lookup: !settings.ignore_system_version,
            allow_binary_download: true,
            use_paths: settings
                .path
                .as_ref()
                .map(|node_path| {
                    let node_path = PathBuf::from(shellexpand::tilde(node_path).as_ref());
                    let npm_path = settings
                        .npm_path
                        .as_ref()
                        .map(|path| PathBuf::from(shellexpand::tilde(&path).as_ref()));
                    (
                        node_path.clone(),
                        npm_path.unwrap_or_else(|| {
                            let base_path = PathBuf::new();
                            node_path.parent().unwrap_or(&base_path).join("npm")
                        }),
                    )
                }),
        };
        tx.send(Some(options)).ok();
    })
    .detach();

    let node_runtime = NodeRuntime::new(client.http_client(), None, rx);

    let prompt_builder = Arc::new(PromptBuilder::new(None).unwrap_or_else(|_| PromptBuilder::new(None).unwrap()));

    Arc::new(AgentAppState {
        client,
        fs,
        languages,
        user_store,
        node_runtime,
        prompt_builder,
    })
}

fn load_model(name: &str, cx: &mut App) -> anyhow::Result<ConfiguredModel> {
    let (provider_name, model_name) = name
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("Model name must be in the format 'provider/model'"))?;

    let provider_id = LanguageModelProviderId::from(provider_name.to_string());
    let provider = LanguageModelRegistry::global(cx)
        .read(cx)
        .provider(&provider_id)
        .ok_or_else(|| anyhow::anyhow!("Provider {} not found", provider_name))?;

    let model = LanguageModelRegistry::global(cx)
        .read(cx)
        .available_models(cx)
        .find(|model| model.name().0 == model_name && model.provider_id() == provider_id)
        .ok_or_else(|| anyhow::anyhow!("Model {} not found for provider {}", model_name, provider_name))?;

    Ok(ConfiguredModel {
        provider,
        model,
    })
}