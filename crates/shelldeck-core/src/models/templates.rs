use crate::models::script::{
    InstallCommand, PackageManager, ScriptCategory, ScriptLanguage, ScriptTarget, ScriptVariable,
    ToolDependency,
};
use serde::{Deserialize, Serialize};

/// A built-in script template that users can import into their script library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub language: ScriptLanguage,
    pub category: ScriptCategory,
    pub dependencies: Vec<ToolDependency>,
    pub tags: Vec<String>,
    pub variables: Vec<ScriptVariable>,
}

impl ScriptTemplate {
    fn new(
        id: &str,
        name: &str,
        description: &str,
        body: &str,
        language: ScriptLanguage,
        category: ScriptCategory,
    ) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            body: body.to_string(),
            language,
            category,
            dependencies: Vec::new(),
            tags: Vec::new(),
            variables: Vec::new(),
        }
    }

    fn with_var(mut self, name: &str, label: &str, default: &str) -> Self {
        self.variables.push(ScriptVariable {
            name: name.to_string(),
            label: Some(label.to_string()),
            description: None,
            default_value: if default.is_empty() {
                None
            } else {
                Some(default.to_string())
            },
        });
        self
    }

    fn with_dep(mut self, name: &str, check: &str, installs: Vec<(&str, PackageManager)>) -> Self {
        self.dependencies.push(ToolDependency {
            name: name.to_string(),
            check_command: check.to_string(),
            install_commands: installs
                .into_iter()
                .map(|(cmd, pm)| InstallCommand {
                    package_manager: pm,
                    command: cmd.to_string(),
                })
                .collect(),
        });
        self
    }

    /// Materialize this template into a `Script` (with a new UUID).
    pub fn to_script(&self) -> super::script::Script {
        let mut script = super::script::Script::new_with_language(
            self.name.clone(),
            self.body.clone(),
            ScriptTarget::AskOnRun,
            self.language.clone(),
            self.category,
        );
        script.dependencies = self.dependencies.clone();
        script.tags = self.tags.clone();
        script.description = Some(self.description.clone());
        script.is_template = false;
        script.template_id = Some(self.id.clone());
        script.variables = self.variables.clone();
        script
    }
}

/// Return all built-in script templates (~40).
pub fn all_templates() -> Vec<ScriptTemplate> {
    let mut t = Vec::with_capacity(42);

    // =====================================================================
    // System (7)
    // =====================================================================

    t.push(ScriptTemplate::new(
        "sys-disk-usage",
        "Disk Usage",
        "Show disk usage in human-readable format",
        "df -h",
        ScriptLanguage::Shell,
        ScriptCategory::System,
    ));

    t.push(ScriptTemplate::new(
        "sys-memory",
        "Memory Usage",
        "Display memory usage statistics",
        "free -h && echo '' && echo '=== Top Memory Consumers ===' && ps aux --sort=-%mem | head -11",
        ScriptLanguage::Shell,
        ScriptCategory::System,
    ));

    t.push(ScriptTemplate::new(
        "sys-cpu",
        "CPU Info & Load",
        "Show CPU information and current load average",
        "echo '=== CPU Info ===' && nproc && lscpu | grep 'Model name' && echo '' && echo '=== Load Average ===' && uptime && echo '' && echo '=== Top CPU Consumers ===' && ps aux --sort=-%cpu | head -11",
        ScriptLanguage::Shell,
        ScriptCategory::System,
    ));

    t.push(ScriptTemplate::new(
        "sys-info",
        "System Info",
        "Comprehensive system overview",
        "echo '=== OS ===' && uname -a && echo '' && echo '=== Hostname ===' && hostname && echo '' && echo '=== Memory ===' && free -h && echo '' && echo '=== Disk ===' && df -h / && echo '' && echo '=== Uptime ===' && uptime",
        ScriptLanguage::Shell,
        ScriptCategory::System,
    ));

    t.push(ScriptTemplate::new(
        "sys-tail-logs",
        "Tail System Logs",
        "Stream system log output (last 50 lines)",
        "tail -n 50 /var/log/syslog 2>/dev/null || tail -n 50 /var/log/messages 2>/dev/null || journalctl -n 50 --no-pager",
        ScriptLanguage::Shell,
        ScriptCategory::System,
    ));

    t.push(ScriptTemplate::new(
        "sys-processes",
        "Top Processes",
        "List top processes by CPU and memory usage",
        "echo '=== By CPU ===' && ps aux --sort=-%cpu | head -11 && echo '' && echo '=== By Memory ===' && ps aux --sort=-%mem | head -11",
        ScriptLanguage::Shell,
        ScriptCategory::System,
    ));

    t.push(ScriptTemplate::new(
        "sys-updates",
        "Check Updates",
        "Check for available package updates",
        "if command -v apt-get >/dev/null 2>&1; then apt list --upgradable 2>/dev/null; elif command -v yum >/dev/null 2>&1; then yum check-update; elif command -v dnf >/dev/null 2>&1; then dnf check-update; elif command -v pacman >/dev/null 2>&1; then pacman -Qu; else echo 'Unknown package manager'; fi",
        ScriptLanguage::Shell,
        ScriptCategory::System,
    ));

    // =====================================================================
    // Network (1)
    // =====================================================================

    t.push(ScriptTemplate::new(
        "net-open-ports",
        "Open Ports",
        "List all open/listening ports",
        "ss -tlnp 2>/dev/null || netstat -tlnp 2>/dev/null",
        ScriptLanguage::Shell,
        ScriptCategory::Network,
    ));

    // =====================================================================
    // Security (1)
    // =====================================================================

    t.push(ScriptTemplate::new(
        "sec-recent-logins",
        "Recent Logins",
        "Show recent login history and failed attempts",
        "echo '=== Last Logins ===' && last -n 20 && echo '' && echo '=== Failed Logins ===' && lastb -n 10 2>/dev/null || echo '(requires root)'",
        ScriptLanguage::Shell,
        ScriptCategory::Security,
    ));

    // =====================================================================
    // Database / MySQL (6)
    // =====================================================================

    t.push(ScriptTemplate::new(
        "mysql-list-dbs",
        "MySQL: List Databases",
        "Show all MySQL databases with sizes",
        "SELECT table_schema AS 'Database', ROUND(SUM(data_length + index_length) / 1024 / 1024, 2) AS 'Size (MB)' FROM information_schema.tables GROUP BY table_schema ORDER BY SUM(data_length + index_length) DESC;",
        ScriptLanguage::Mysql,
        ScriptCategory::Database,
    ).with_dep("mysql", "which mysql", vec![
        ("sudo apt-get install -y mysql-client", PackageManager::Apt),
        ("sudo yum install -y mysql", PackageManager::Yum),
        ("sudo dnf install -y mysql", PackageManager::Dnf),
    ]));

    t.push(
        ScriptTemplate::new(
            "mysql-backup",
            "MySQL: Backup Database",
            "Dump a MySQL database",
            "mysqldump --single-transaction --routines --triggers {{database}}",
            ScriptLanguage::Shell,
            ScriptCategory::Database,
        )
        .with_var("database", "Database Name", "mydb")
        .with_dep(
            "mysqldump",
            "which mysqldump",
            vec![("sudo apt-get install -y mysql-client", PackageManager::Apt)],
        ),
    );

    t.push(
        ScriptTemplate::new(
            "mysql-users",
            "MySQL: List Users",
            "Show all MySQL users and their hosts",
            "SELECT User, Host, plugin FROM mysql.user ORDER BY User;",
            ScriptLanguage::Mysql,
            ScriptCategory::Database,
        )
        .with_dep(
            "mysql",
            "which mysql",
            vec![("sudo apt-get install -y mysql-client", PackageManager::Apt)],
        ),
    );

    t.push(ScriptTemplate::new(
        "mysql-slow-queries",
        "MySQL: Slow Queries",
        "Show recent slow queries",
        "SELECT start_time, query_time, lock_time, rows_examined, sql_text FROM mysql.slow_log ORDER BY start_time DESC LIMIT 20;",
        ScriptLanguage::Mysql,
        ScriptCategory::Database,
    ).with_dep("mysql", "which mysql", vec![
        ("sudo apt-get install -y mysql-client", PackageManager::Apt),
    ]));

    t.push(
        ScriptTemplate::new(
            "mysql-processlist",
            "MySQL: Process List",
            "Show currently running MySQL queries",
            "SHOW FULL PROCESSLIST;",
            ScriptLanguage::Mysql,
            ScriptCategory::Database,
        )
        .with_dep(
            "mysql",
            "which mysql",
            vec![("sudo apt-get install -y mysql-client", PackageManager::Apt)],
        ),
    );

    t.push(ScriptTemplate::new(
        "mysql-table-sizes",
        "MySQL: Table Sizes",
        "Show largest tables across all databases",
        "SELECT table_schema, table_name, ROUND((data_length + index_length) / 1024 / 1024, 2) AS 'Size (MB)', table_rows AS 'Rows' FROM information_schema.tables ORDER BY (data_length + index_length) DESC LIMIT 20;",
        ScriptLanguage::Mysql,
        ScriptCategory::Database,
    ).with_dep("mysql", "which mysql", vec![
        ("sudo apt-get install -y mysql-client", PackageManager::Apt),
    ]));

    // =====================================================================
    // Database / PostgreSQL (5)
    // =====================================================================

    t.push(ScriptTemplate::new(
        "pg-list-dbs",
        "PostgreSQL: List Databases",
        "Show all PostgreSQL databases with sizes",
        "SELECT datname AS database, pg_size_pretty(pg_database_size(datname)) AS size FROM pg_database WHERE datistemplate = false ORDER BY pg_database_size(datname) DESC;",
        ScriptLanguage::Postgresql,
        ScriptCategory::Database,
    ).with_dep("psql", "which psql", vec![
        ("sudo apt-get install -y postgresql-client", PackageManager::Apt),
        ("sudo yum install -y postgresql", PackageManager::Yum),
    ]));

    t.push(
        ScriptTemplate::new(
            "pg-backup",
            "PostgreSQL: Backup Database",
            "Dump a PostgreSQL database",
            "pg_dump --format=custom --verbose {{database}}",
            ScriptLanguage::Shell,
            ScriptCategory::Database,
        )
        .with_var("database", "Database Name", "mydb")
        .with_dep(
            "pg_dump",
            "which pg_dump",
            vec![(
                "sudo apt-get install -y postgresql-client",
                PackageManager::Apt,
            )],
        ),
    );

    t.push(ScriptTemplate::new(
        "pg-active-queries",
        "PostgreSQL: Active Queries",
        "Show currently running queries",
        "SELECT pid, now() - pg_stat_activity.query_start AS duration, query, state FROM pg_stat_activity WHERE (now() - pg_stat_activity.query_start) > interval '1 second' ORDER BY duration DESC;",
        ScriptLanguage::Postgresql,
        ScriptCategory::Database,
    ).with_dep("psql", "which psql", vec![
        ("sudo apt-get install -y postgresql-client", PackageManager::Apt),
    ]));

    t.push(ScriptTemplate::new(
        "pg-table-sizes",
        "PostgreSQL: Table Sizes",
        "Show largest tables in current database",
        "SELECT schemaname, tablename, pg_size_pretty(pg_total_relation_size(schemaname || '.' || tablename)) AS size FROM pg_tables WHERE schemaname NOT IN ('pg_catalog', 'information_schema') ORDER BY pg_total_relation_size(schemaname || '.' || tablename) DESC LIMIT 20;",
        ScriptLanguage::Postgresql,
        ScriptCategory::Database,
    ).with_dep("psql", "which psql", vec![
        ("sudo apt-get install -y postgresql-client", PackageManager::Apt),
    ]));

    t.push(ScriptTemplate::new(
        "pg-connections",
        "PostgreSQL: Connection Stats",
        "Show connection statistics per database",
        "SELECT datname, numbackends AS connections, xact_commit AS commits, xact_rollback AS rollbacks, blks_hit, blks_read FROM pg_stat_database WHERE datname IS NOT NULL ORDER BY numbackends DESC;",
        ScriptLanguage::Postgresql,
        ScriptCategory::Database,
    ).with_dep("psql", "which psql", vec![
        ("sudo apt-get install -y postgresql-client", PackageManager::Apt),
    ]));

    // =====================================================================
    // Web / Nginx (4)
    // =====================================================================

    t.push(
        ScriptTemplate::new(
            "nginx-test",
            "Nginx: Config Test",
            "Test nginx configuration for syntax errors",
            "-t",
            ScriptLanguage::Nginx,
            ScriptCategory::Web,
        )
        .with_dep(
            "nginx",
            "which nginx",
            vec![("sudo apt-get install -y nginx", PackageManager::Apt)],
        ),
    );

    t.push(
        ScriptTemplate::new(
            "nginx-reload",
            "Nginx: Reload",
            "Reload nginx configuration without downtime",
            "-s reload",
            ScriptLanguage::Nginx,
            ScriptCategory::Web,
        )
        .with_dep(
            "nginx",
            "which nginx",
            vec![("sudo apt-get install -y nginx", PackageManager::Apt)],
        ),
    );

    t.push(ScriptTemplate::new(
        "nginx-sites",
        "Nginx: List Sites",
        "Show enabled nginx sites",
        "ls -la /etc/nginx/sites-enabled/ 2>/dev/null && echo '' && echo '=== Config Test ===' && nginx -t 2>&1",
        ScriptLanguage::Shell,
        ScriptCategory::Web,
    ).with_dep("nginx", "which nginx", vec![
        ("sudo apt-get install -y nginx", PackageManager::Apt),
    ]));

    t.push(ScriptTemplate::new(
        "nginx-access-log",
        "Nginx: Access Log (last 50)",
        "Show last 50 lines of the nginx access log",
        "tail -n 50 /var/log/nginx/access.log 2>/dev/null || echo 'Access log not found'",
        ScriptLanguage::Shell,
        ScriptCategory::Web,
    ));

    // =====================================================================
    // Web / PHP (4)
    // =====================================================================

    t.push(
        ScriptTemplate::new(
            "php-version",
            "PHP: Version Info",
            "Show PHP version and loaded modules",
            "echo php_uname(); echo PHP_VERSION; echo implode(', ', get_loaded_extensions());",
            ScriptLanguage::Php,
            ScriptCategory::Web,
        )
        .with_dep(
            "php",
            "which php",
            vec![("sudo apt-get install -y php-cli", PackageManager::Apt)],
        ),
    );

    t.push(
        ScriptTemplate::new(
            "php-laravel-migrate",
            "Laravel: Run Migrations",
            "Run Laravel database migrations",
            "cd {{project_path}} && php artisan migrate --force",
            ScriptLanguage::Shell,
            ScriptCategory::Web,
        )
        .with_var("project_path", "Project Path", "/var/www/html")
        .with_dep(
            "php",
            "which php",
            vec![("sudo apt-get install -y php-cli", PackageManager::Apt)],
        ),
    );

    t.push(ScriptTemplate::new(
        "php-cache-clear",
        "Laravel: Clear Cache",
        "Clear all Laravel caches",
        "cd {{project_path}} && php artisan cache:clear && php artisan config:clear && php artisan route:clear && php artisan view:clear && echo 'All caches cleared'",
        ScriptLanguage::Shell,
        ScriptCategory::Web,
    ).with_var("project_path", "Project Path", "/var/www/html")
    .with_dep("php", "which php", vec![
        ("sudo apt-get install -y php-cli", PackageManager::Apt),
    ]));

    t.push(
        ScriptTemplate::new(
            "php-composer-install",
            "Composer: Install",
            "Install PHP dependencies via Composer",
            "cd {{project_path}} && composer install --no-dev --optimize-autoloader",
            ScriptLanguage::Shell,
            ScriptCategory::Web,
        )
        .with_var("project_path", "Project Path", "/var/www/html")
        .with_dep(
            "composer",
            "which composer",
            vec![("sudo apt-get install -y composer", PackageManager::Apt)],
        ),
    );

    // =====================================================================
    // Runtime / Bun + Node (4)
    // =====================================================================

    t.push(
        ScriptTemplate::new(
            "pm2-status",
            "PM2: Process Status",
            "Show PM2 managed process status",
            "pm2 list && echo '' && pm2 monit --no-color 2>/dev/null | head -30 || true",
            ScriptLanguage::Shell,
            ScriptCategory::Runtime,
        )
        .with_dep(
            "pm2",
            "which pm2",
            vec![("sudo npm install -g pm2", PackageManager::Apt)],
        ),
    );

    t.push(ScriptTemplate::new(
        "node-install-deps",
        "Node: Install Dependencies",
        "Install Node.js dependencies in current project",
        "if [ -f bun.lockb ]; then bun install; elif [ -f yarn.lock ]; then yarn install; elif [ -f pnpm-lock.yaml ]; then pnpm install; else npm install; fi",
        ScriptLanguage::Shell,
        ScriptCategory::Runtime,
    ).with_dep("node", "which node", vec![
        ("sudo apt-get install -y nodejs npm", PackageManager::Apt),
    ]));

    t.push(
        ScriptTemplate::new(
            "bun-run",
            "Bun: Run Script",
            "Run a script with Bun (edit script name)",
            "console.log('Hello from Bun!'); console.log(`Bun version: ${Bun.version}`);",
            ScriptLanguage::Bun,
            ScriptCategory::Runtime,
        )
        .with_dep("bun", "which bun", vec![]),
    );

    t.push(ScriptTemplate::new(
        "node-version-check",
        "Node: Version Check",
        "Check installed Node.js, npm, and related tool versions",
        "echo \"Node: $(node --version 2>/dev/null || echo 'not installed')\" && echo \"npm: $(npm --version 2>/dev/null || echo 'not installed')\" && echo \"Bun: $(bun --version 2>/dev/null || echo 'not installed')\" && echo \"Yarn: $(yarn --version 2>/dev/null || echo 'not installed')\" && echo \"pnpm: $(pnpm --version 2>/dev/null || echo 'not installed')\"",
        ScriptLanguage::Shell,
        ScriptCategory::Runtime,
    ));

    // =====================================================================
    // Container / Docker (5)
    // =====================================================================

    t.push(
        ScriptTemplate::new(
            "docker-running",
            "Docker: Running Containers",
            "List all running Docker containers",
            "ps --format 'table {{.ID}}\t{{.Names}}\t{{.Status}}\t{{.Ports}}'",
            ScriptLanguage::Docker,
            ScriptCategory::Container,
        )
        .with_dep(
            "docker",
            "which docker",
            vec![("sudo apt-get install -y docker.io", PackageManager::Apt)],
        ),
    );

    t.push(
        ScriptTemplate::new(
            "docker-all",
            "Docker: All Containers",
            "List all Docker containers including stopped",
            "ps -a --format 'table {{.ID}}\t{{.Names}}\t{{.Status}}\t{{.Image}}'",
            ScriptLanguage::Docker,
            ScriptCategory::Container,
        )
        .with_dep(
            "docker",
            "which docker",
            vec![("sudo apt-get install -y docker.io", PackageManager::Apt)],
        ),
    );

    t.push(
        ScriptTemplate::new(
            "docker-images",
            "Docker: List Images",
            "List all Docker images with sizes",
            "images --format 'table {{.Repository}}\t{{.Tag}}\t{{.Size}}\t{{.CreatedSince}}'",
            ScriptLanguage::Docker,
            ScriptCategory::Container,
        )
        .with_dep(
            "docker",
            "which docker",
            vec![("sudo apt-get install -y docker.io", PackageManager::Apt)],
        ),
    );

    t.push(
        ScriptTemplate::new(
            "docker-compose-status",
            "Docker Compose: Status",
            "Show Docker Compose service status",
            "ps --format table",
            ScriptLanguage::DockerCompose,
            ScriptCategory::Container,
        )
        .with_dep(
            "docker",
            "which docker",
            vec![("sudo apt-get install -y docker.io", PackageManager::Apt)],
        ),
    );

    t.push(
        ScriptTemplate::new(
            "docker-logs",
            "Docker: Container Logs",
            "Show last 50 lines of a container's logs",
            "logs --tail 50 {{container}}",
            ScriptLanguage::Docker,
            ScriptCategory::Container,
        )
        .with_var("container", "Container Name", "")
        .with_dep(
            "docker",
            "which docker",
            vec![("sudo apt-get install -y docker.io", PackageManager::Apt)],
        ),
    );

    // =====================================================================
    // System / Systemd (3)
    // =====================================================================

    t.push(
        ScriptTemplate::new(
            "systemd-status",
            "Systemd: Service Status",
            "Check status of a service",
            "status {{service}}",
            ScriptLanguage::Systemd,
            ScriptCategory::System,
        )
        .with_var("service", "Service Name", "nginx"),
    );

    t.push(
        ScriptTemplate::new(
            "systemd-restart",
            "Systemd: Restart Service",
            "Restart a systemd service",
            "restart {{service}}",
            ScriptLanguage::Systemd,
            ScriptCategory::System,
        )
        .with_var("service", "Service Name", "nginx"),
    );

    t.push(ScriptTemplate::new(
        "systemd-enabled",
        "Systemd: List Enabled Services",
        "Show all enabled systemd services",
        "list-unit-files --state=enabled --type=service --no-pager",
        ScriptLanguage::Systemd,
        ScriptCategory::System,
    ));

    // =====================================================================
    // Python (1)
    // =====================================================================

    t.push(ScriptTemplate::new(
        "python-sysinfo",
        "Python: System Info",
        "Gather system information via Python",
        "import platform, os\nprint(f'OS: {platform.system()} {platform.release()}')\nprint(f'Python: {platform.python_version()}')\nprint(f'CPU cores: {os.cpu_count()}')\nprint(f'User: {os.getenv(\"USER\", \"unknown\")}')",
        ScriptLanguage::Python,
        ScriptCategory::System,
    ).with_dep("python3", "which python3", vec![
        ("sudo apt-get install -y python3", PackageManager::Apt),
    ]));

    t
}
