use anyhow::{Result, anyhow};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use crate::audit::AuditLogger;
use crate::confirmation::ConfirmationManager;
use crate::connection::{ConnectionManager, ProxmoxClient};
use crate::tools::{truncate_output, wrap_output_envelope};

const OUTPUT_MAX_CHARS: usize = 10_000;
const DEFAULT_TASK_WAIT_TIMEOUT_SECS: u64 = 600;

#[derive(Clone, Copy)]
struct AutoVmidRetryPolicy {
    attempts: usize,
    backoff_ms: u64,
}

pub async fn node_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let nodes = client.get("/nodes").await?;

        let nodes = nodes
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected response format from /nodes"))?;

        if nodes.is_empty() {
            return Ok("No nodes found.".to_string());
        }

        let mut lines = vec![format!(
            "Proxmox host: {}\n\n{:<16}  {:<10}  {:<8}  {:<8}  {:<12}  {}",
            host, "NODE", "STATUS", "CPU", "MEM %", "UPTIME", "PVE VERSION"
        )];

        for node in nodes {
            let name = node.get("node").and_then(Value::as_str).unwrap_or("?");
            let status = node.get("status").and_then(Value::as_str).unwrap_or("?");
            let cpu = node.get("cpu").and_then(Value::as_f64).unwrap_or(0.0);
            let maxmem = node.get("maxmem").and_then(Value::as_u64).unwrap_or(1);
            let mem = node.get("mem").and_then(Value::as_u64).unwrap_or(0);
            let mem_pct = if maxmem > 0 {
                (mem as f64 / maxmem as f64) * 100.0
            } else {
                0.0
            };
            let uptime = node.get("uptime").and_then(Value::as_u64).unwrap_or(0);
            let pve_version = node
                .get("pveversion")
                .and_then(Value::as_str)
                .unwrap_or("?");

            lines.push(format!(
                "{:<16}  {:<10}  {:<8.1}%  {:<8.1}%  {:<12}  {}",
                name,
                status,
                cpu * 100.0,
                mem_pct,
                format_uptime(uptime),
                pve_version
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.node.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.node.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.node.list",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn node_status(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let status = client.get(&format!("/nodes/{}/status", node_name)).await?;

        let cpu_model = status
            .pointer("/cpuinfo/model")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let cpu_cores = status
            .pointer("/cpuinfo/cpus")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cpu_sockets = status
            .pointer("/cpuinfo/sockets")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cpu_usage = status.get("cpu").and_then(Value::as_f64).unwrap_or(0.0);
        let mem_total = status
            .pointer("/memory/total")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mem_used = status
            .pointer("/memory/used")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mem_free = status
            .pointer("/memory/free")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let swap_total = status
            .pointer("/swap/total")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let swap_used = status
            .pointer("/swap/used")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let rootfs_total = status
            .pointer("/rootfs/total")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let rootfs_used = status
            .pointer("/rootfs/used")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let uptime = status.get("uptime").and_then(Value::as_u64).unwrap_or(0);
        let pve_version = status
            .get("pveversion")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let kernel = status
            .get("kversion")
            .and_then(Value::as_str)
            .unwrap_or("?");

        let output = format!(
            "Node: {} (Proxmox host: {})\n\
             PVE Version: {}\n\
             Kernel: {}\n\
             Uptime: {}\n\n\
             CPU: {} ({} sockets, {} cores)\n\
             CPU Usage: {:.1}%\n\
             Load Average: {}\n\n\
             Memory: {} / {} ({:.1}% used)\n\
             Free: {}\n\
             Swap: {} / {}\n\n\
             Root FS: {} / {} ({:.1}% used)",
            node_name,
            host,
            pve_version,
            kernel,
            format_uptime(uptime),
            cpu_model,
            cpu_sockets,
            cpu_cores,
            cpu_usage * 100.0,
            format_loadavg(status.get("loadavg")),
            format_bytes(mem_used),
            format_bytes(mem_total),
            percent(mem_used, mem_total),
            format_bytes(mem_free),
            format_bytes(swap_used),
            format_bytes(swap_total),
            format_bytes(rootfs_used),
            format_bytes(rootfs_total),
            percent(rootfs_used, rootfs_total),
        );

        Ok(output)
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.node.status", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.node.status", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.node.status",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vm_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let filter_type = vm_type.as_deref();

        if let Some(filter_type) = filter_type {
            ensure_vm_type(filter_type)?;
        }

        let mut all_vms: Vec<Value> = Vec::new();

        if filter_type.is_none() || filter_type == Some("qemu") {
            let qemu = client.get(&format!("/nodes/{}/qemu", node_name)).await?;
            if let Some(items) = qemu.as_array() {
                for mut vm in items.clone() {
                    if let Some(object) = vm.as_object_mut() {
                        object.insert("type".to_string(), Value::String("qemu".to_string()));
                    }
                    all_vms.push(vm);
                }
            }
        }

        if filter_type.is_none() || filter_type == Some("lxc") {
            let lxc = client.get(&format!("/nodes/{}/lxc", node_name)).await?;
            if let Some(items) = lxc.as_array() {
                for mut vm in items.clone() {
                    if let Some(object) = vm.as_object_mut() {
                        object.insert("type".to_string(), Value::String("lxc".to_string()));
                    }
                    all_vms.push(vm);
                }
            }
        }

        if all_vms.is_empty() {
            return Ok(format!("No VMs/CTs found on node '{}'.", node_name));
        }

        all_vms.sort_by_key(|vm| vm.get("vmid").and_then(Value::as_u64).unwrap_or(0));

        let mut lines = vec![format!(
            "Node: {} (Proxmox host: {})\n\n{:<8}  {:<6}  {:<24}  {:<10}  {:<6}  {:<10}  {}",
            node_name, host, "VMID", "TYPE", "NAME", "STATUS", "CPU", "MEM", "UPTIME"
        )];

        for vm in &all_vms {
            let vmid = vm.get("vmid").and_then(Value::as_u64).unwrap_or(0);
            let vm_type = vm.get("type").and_then(Value::as_str).unwrap_or("?");
            let name = vm.get("name").and_then(Value::as_str).unwrap_or("?");
            let status = vm.get("status").and_then(Value::as_str).unwrap_or("?");
            let cpus = vm.get("cpus").and_then(Value::as_u64).unwrap_or(0);
            let maxmem = vm.get("maxmem").and_then(Value::as_u64).unwrap_or(0);
            let uptime = vm.get("uptime").and_then(Value::as_u64).unwrap_or(0);

            lines.push(format!(
                "{:<8}  {:<6}  {:<24}  {:<10}  {:<6}  {:<10}  {}",
                vmid,
                vm_type,
                name,
                status,
                cpus,
                format_bytes(maxmem),
                if uptime > 0 {
                    format_uptime(uptime)
                } else {
                    "-".to_string()
                }
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.vm.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.list", &output))
        }
        Err(error) => {
            audit
                .log("proxmox.vm.list", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_status(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(vm_type.as_deref())?;
        let path = format!("/nodes/{}/{}/{}/status/current", node_name, vm_type, vmid);
        let status = client.get(&path).await?;

        let name = status.get("name").and_then(Value::as_str).unwrap_or("?");
        let vm_status = status.get("status").and_then(Value::as_str).unwrap_or("?");
        let cpus = status.get("cpus").and_then(Value::as_u64).unwrap_or(0);
        let cpu_usage = status.get("cpu").and_then(Value::as_f64).unwrap_or(0.0);
        let maxmem = status.get("maxmem").and_then(Value::as_u64).unwrap_or(0);
        let mem = status.get("mem").and_then(Value::as_u64).unwrap_or(0);
        let maxdisk = status.get("maxdisk").and_then(Value::as_u64).unwrap_or(0);
        let disk = status.get("disk").and_then(Value::as_u64).unwrap_or(0);
        let uptime = status.get("uptime").and_then(Value::as_u64).unwrap_or(0);
        let netin = status.get("netin").and_then(Value::as_u64).unwrap_or(0);
        let netout = status.get("netout").and_then(Value::as_u64).unwrap_or(0);
        let pid = status.get("pid").and_then(Value::as_u64);
        let qmpstatus = status.get("qmpstatus").and_then(Value::as_str);

        let mut output = format!(
            "VM/CT {} ({}) on node {} (Proxmox host: {})\n\n\
             Name: {}\n\
             Type: {}\n\
             Status: {}{}\n\
             Uptime: {}\n\n\
             CPU: {} cores, {:.1}% usage\n\
             Memory: {} / {} ({:.1}% used)\n\
             Disk: {} / {} ({:.1}% used)\n\n\
             Network In: {}\n\
             Network Out: {}",
            vmid,
            vm_type,
            node_name,
            host,
            name,
            vm_type,
            vm_status,
            qmpstatus
                .map(|status| format!(" (QMP: {})", status))
                .unwrap_or_default(),
            format_uptime(uptime),
            cpus,
            cpu_usage * 100.0,
            format_bytes(mem),
            format_bytes(maxmem),
            percent(mem, maxmem),
            format_bytes(disk),
            format_bytes(maxdisk),
            percent(disk, maxdisk),
            format_bytes(netin),
            format_bytes(netout),
        );

        if let Some(pid) = pid {
            output.push_str(&format!("\nPID: {}", pid));
        }

        Ok(output)
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.status",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.status", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.status",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_start(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would start {} {} on Proxmox host '{}'.",
            vm_type, vmid, host
        );
        audit
            .log(
                "proxmox.vm.start",
                &host,
                "dry_run",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.start", &output));
    }

    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let path = format!("/nodes/{}/{}/{}/status/start", node_name, vm_type, vmid);
        let response = client.post(&path, &[]).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, task_wait_timeout_secs(&manager, &host))
                .await?;
            Ok(format!(
                "Started {} {} on node '{}'. {}",
                vm_type, vmid, node_name, result
            ))
        } else {
            Ok(format!(
                "Started {} {} on node '{}'.",
                vm_type, vmid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.start",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.start", &output))
        }
        Err(error) => {
            audit
                .log("proxmox.vm.start", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn vm_stop(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would force-stop {} {} on Proxmox host '{}'.",
            vm_type, vmid, host
        );
        audit
            .log("proxmox.vm.stop", &host, "dry_run", Some(&vmid.to_string()))
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.stop", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "vmid": vmid,
        "vm_type": vm_type.clone(),
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.stop",
            None,
            &format!(
                "About to FORCE-STOP {} {} on Proxmox host '{}'. The VM/CT will be stopped immediately.",
                vm_type, vmid, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.stop",
                &host,
                "confirmation_required",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(response);
    }

    vm_stop_confirmed(manager, host, node, vmid, Some(vm_type), audit).await
}

pub async fn vm_stop_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(vm_type.as_deref())?;
        let path = format!("/nodes/{}/{}/{}/status/stop", node_name, vm_type, vmid);
        let response = client.post(&path, &[]).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, task_wait_timeout_secs(&manager, &host))
                .await?;
            Ok(format!(
                "Force-stopped {} {} on node '{}'. {}",
                vm_type, vmid, node_name, result
            ))
        } else {
            Ok(format!(
                "Force-stop requested for {} {} on node '{}'.",
                vm_type, vmid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.vm.stop", &host, "success", Some(&vmid.to_string()))
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.stop", &output))
        }
        Err(error) => {
            audit
                .log("proxmox.vm.stop", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn vm_create(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: Option<u64>,
    vm_type: Option<String>,
    name: Option<String>,
    cores: Option<u64>,
    memory: Option<u64>,
    os_type: Option<String>,
    template: Option<String>,
    iso: Option<String>,
    storage: Option<String>,
    disk_size: Option<String>,
    net: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;
    validate_vm_create_platform_inputs(&vm_type, os_type.as_deref(), template.as_deref())?;

    let mut param_lines = vec![format!("Type: {}", vm_type)];
    if let Some(vmid) = vmid {
        param_lines.push(format!("VMID: {}", vmid));
    }
    if let Some(name) = &name {
        param_lines.push(format!("Name: {}", name));
    }
    if let Some(cores) = cores {
        param_lines.push(format!("Cores: {}", cores));
    }
    if let Some(memory) = memory {
        param_lines.push(format!("Memory: {} MB", memory));
    }
    if vm_type == "qemu" {
        if let Some(os_type) = &os_type {
            param_lines.push(format!("OS Type: {}", os_type));
        }
    } else if let Some(template) = &template {
        param_lines.push(format!("Template: {}", template));
    }
    if let Some(iso) = &iso {
        param_lines.push(format!("ISO: {}", iso));
    }
    if let Some(storage) = &storage {
        param_lines.push(format!("Storage: {}", storage));
    }
    if let Some(disk_size) = &disk_size {
        param_lines.push(format!("Disk: {}", disk_size));
    }
    if let Some(net) = &net {
        param_lines.push(format!("Network: {}", net));
    }

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would create {} on Proxmox host '{}':\n{}",
            vm_type,
            host,
            param_lines.join("\n")
        );
        audit
            .log("proxmox.vm.create", &host, "dry_run", name.as_deref())
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.create", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "vmid": vmid,
        "vm_type": vm_type.clone(),
        "name": name.clone(),
        "cores": cores,
        "memory": memory,
        "os_type": os_type.clone(),
        "template": template.clone(),
        "iso": iso.clone(),
        "storage": storage.clone(),
        "disk_size": disk_size.clone(),
        "net": net.clone(),
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.create",
            None,
            &format!(
                "About to CREATE a new {} on Proxmox host '{}':\n{}",
                vm_type,
                host,
                param_lines.join("\n")
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.create",
                &host,
                "confirmation_required",
                name.as_deref(),
            )
            .await
            .ok();
        return Ok(response);
    }

    vm_create_confirmed(
        manager, host, node, vmid, vm_type, name, cores, memory, os_type, template, iso, storage,
        disk_size, net, audit,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn vm_create_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    vmid: Option<u64>,
    vm_type: String,
    name: Option<String>,
    cores: Option<u64>,
    memory: Option<u64>,
    os_type: Option<String>,
    template: Option<String>,
    iso: Option<String>,
    storage: Option<String>,
    disk_size: Option<String>,
    net: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(Some(&vm_type))?;
        validate_vm_create_platform_inputs(vm_type, os_type.as_deref(), template.as_deref())?;
        let retry_policy = auto_vmid_retry_policy(&manager, &host);
        let auto_vmid = vmid.is_none();
        let mut vmid = match vmid {
            Some(vmid) => vmid,
            None => next_vmid(&client).await?,
        };
        let path = format!("/nodes/{}/{}", node_name, vm_type);
        let mut conflict_retries = 0usize;

        loop {
            let mut params: Vec<(&str, String)> = vec![("vmid", vmid.to_string())];

            if let Some(name) = &name {
                params.push(("name", name.clone()));
            }
            if let Some(cores) = cores {
                params.push(("cores", cores.to_string()));
            }
            if let Some(memory) = memory {
                params.push(("memory", memory.to_string()));
            }

            if vm_type == "qemu" {
                if let Some(os_type) = &os_type {
                    params.push(("ostype", os_type.clone()));
                }
                if let Some(iso) = &iso {
                    params.push(("ide2", format!("{},media=cdrom", iso)));
                }
                if let Some(storage) = &storage {
                    let size = disk_size.as_deref().unwrap_or("32G");
                    params.push(("scsi0", format!("{}:{}", storage, size)));
                }
                if let Some(net) = &net {
                    params.push(("net0", net.clone()));
                }
            } else {
                if let Some(template) = &template {
                    params.push(("ostemplate", template.clone()));
                }
                if let Some(storage) = &storage {
                    let size = disk_size.as_deref().unwrap_or("8");
                    params.push(("rootfs", format!("{}:{}", storage, size)));
                }
                if let Some(net) = &net {
                    params.push(("net0", net.clone()));
                }
            }

            let param_refs: Vec<(&str, &str)> = params
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect();

            match client.post(&path, &param_refs).await {
                Ok(response) => {
                    break if let Some(upid) = response.as_str() {
                        let result = client
                            .wait_for_task(
                                &node_name,
                                upid,
                                task_wait_timeout_secs(&manager, &host),
                            )
                            .await?;
                        Ok(format!(
                            "Created {} {} on node '{}'. {}",
                            vm_type, vmid, node_name, result
                        ))
                    } else {
                        Ok(format!(
                            "Created {} {} on node '{}'.",
                            vm_type, vmid, node_name
                        ))
                    };
                }
                Err(error)
                    if auto_vmid
                        && is_duplicate_vmid_conflict(&error, vmid)
                        && conflict_retries < retry_policy.attempts =>
                {
                    conflict_retries += 1;
                    sleep(Duration::from_millis(retry_policy.backoff_ms)).await;
                    vmid = next_vmid(&client).await?;
                }
                Err(error) if auto_vmid && is_duplicate_vmid_conflict(&error, vmid) => {
                    break Err(exhausted_auto_vmid_error(
                        "proxmox.vm.create",
                        "vmid",
                        retry_policy,
                        error,
                    ));
                }
                Err(error) => break Err(error),
            }
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.vm.create", &host, "success", name.as_deref())
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.create", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.create",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn vm_clone(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    newid: Option<u64>,
    name: Option<String>,
    full: Option<bool>,
    target_storage: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would clone VM {} to new VM{} on Proxmox host '{}'.\n\
             Full clone: {}\n{}",
            vmid,
            newid.map(|id| format!(" {}", id)).unwrap_or_default(),
            host,
            full.unwrap_or(true),
            name.as_ref()
                .map(|name| format!("Name: {}", name))
                .unwrap_or_default(),
        );
        audit
            .log(
                "proxmox.vm.clone",
                &host,
                "dry_run",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.clone", &output));
    }

    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let retry_policy = auto_vmid_retry_policy(&manager, &host);
        let auto_newid = newid.is_none();
        let mut newid = match newid {
            Some(newid) => newid,
            None => next_vmid(&client).await?,
        };
        let path = format!("/nodes/{}/qemu/{}/clone", node_name, vmid);
        let mut conflict_retries = 0usize;

        loop {
            let mut params: Vec<(&str, String)> = vec![("newid", newid.to_string())];
            if let Some(name) = &name {
                params.push(("name", name.clone()));
            }
            if let Some(full) = full {
                params.push(("full", bool_to_proxmox(full).to_string()));
            }
            if let Some(target_storage) = &target_storage {
                params.push(("storage", target_storage.clone()));
            }

            let param_refs: Vec<(&str, &str)> = params
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect();

            match client.post(&path, &param_refs).await {
                Ok(response) => {
                    break if let Some(upid) = response.as_str() {
                        let result = client
                            .wait_for_task(
                                &node_name,
                                upid,
                                task_wait_timeout_secs(&manager, &host),
                            )
                            .await?;
                        Ok(format!(
                            "Cloned VM {} to {} on node '{}'. {}\n{}",
                            vmid,
                            newid,
                            node_name,
                            result,
                            name.as_ref()
                                .map(|name| format!("Name: {}", name))
                                .unwrap_or_default(),
                        ))
                    } else {
                        Ok(format!(
                            "Clone requested for VM {} to {} on node '{}'.",
                            vmid, newid, node_name
                        ))
                    };
                }
                Err(error)
                    if auto_newid
                        && is_duplicate_vmid_conflict(&error, newid)
                        && conflict_retries < retry_policy.attempts =>
                {
                    conflict_retries += 1;
                    sleep(Duration::from_millis(retry_policy.backoff_ms)).await;
                    newid = next_vmid(&client).await?;
                }
                Err(error) if auto_newid && is_duplicate_vmid_conflict(&error, newid) => {
                    break Err(exhausted_auto_vmid_error(
                        "proxmox.vm.clone",
                        "newid",
                        retry_policy,
                        error,
                    ));
                }
                Err(error) => break Err(error),
            }
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.clone",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.clone", &output))
        }
        Err(error) => {
            audit
                .log("proxmox.vm.clone", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn vm_delete(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    purge: Option<bool>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would DELETE {} {} on Proxmox host '{}'. Purge: {}",
            vm_type,
            vmid,
            host,
            purge.unwrap_or(false)
        );
        audit
            .log(
                "proxmox.vm.delete",
                &host,
                "dry_run",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.delete", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "vmid": vmid,
        "vm_type": vm_type.clone(),
        "purge": purge,
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.delete",
            None,
            &format!(
                "About to PERMANENTLY DELETE {} {} on Proxmox host '{}'. This cannot be undone.",
                vm_type, vmid, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.delete",
                &host,
                "confirmation_required",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(response);
    }

    vm_delete_confirmed(manager, host, node, vmid, vm_type, purge, audit).await
}

pub async fn vm_delete_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    vmid: u64,
    vm_type: String,
    purge: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(Some(&vm_type))?;
        let mut path = format!("/nodes/{}/{}/{}", node_name, vm_type, vmid);
        if purge.unwrap_or(false) {
            path.push_str("?purge=1&destroy-unreferenced-disks=1");
        }

        let response = client.delete(&path).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, task_wait_timeout_secs(&manager, &host))
                .await?;
            Ok(format!(
                "Deleted {} {} on node '{}'. {}",
                vm_type, vmid, node_name, result
            ))
        } else {
            Ok(format!(
                "Delete requested for {} {} on node '{}'.",
                vm_type, vmid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.delete",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.delete", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.delete",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn snapshot_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(vm_type.as_deref())?;
        let path = format!("/nodes/{}/{}/{}/snapshot", node_name, vm_type, vmid);
        let snapshots = client.get(&path).await?;

        let snapshots = snapshots
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected response format from snapshot list"))?;

        if snapshots.is_empty()
            || (snapshots.len() == 1
                && snapshots[0].get("name").and_then(Value::as_str) == Some("current"))
        {
            return Ok(format!(
                "No snapshots found for {} {} on node '{}'.",
                vm_type, vmid, node_name
            ));
        }

        let mut lines = vec![format!(
            "Snapshots for {} {} on node '{}' (Proxmox host: {})\n\n{:<24}  {:<24}  {:<10}  {}",
            vm_type, vmid, node_name, host, "NAME", "DESCRIPTION", "RAM", "PARENT"
        )];

        for snapshot in snapshots {
            let name = snapshot.get("name").and_then(Value::as_str).unwrap_or("?");
            if name == "current" {
                continue;
            }

            let description = snapshot
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            let snaptime = snapshot.get("snaptime").and_then(Value::as_u64);
            let vmstate = snapshot.get("vmstate").and_then(Value::as_u64).unwrap_or(0);
            let parent = snapshot
                .get("parent")
                .and_then(Value::as_str)
                .unwrap_or("-");

            let time_str = snaptime
                .and_then(|timestamp| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0)
                })
                .map(|datetime| datetime.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "?".to_string());

            lines.push(format!(
                "{:<24}  {:<24}  {:<10}  {}\n  Created: {}",
                name,
                truncate_cell(description, 24),
                if vmstate > 0 { "yes" } else { "no" },
                parent,
                time_str,
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.snapshot.list",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.snapshot.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.snapshot.list",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn snapshot_create(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    snapname: String,
    description: Option<String>,
    vmstate: Option<bool>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would create snapshot '{}' for {} {} on Proxmox host '{}'.",
            snapname, vm_type, vmid, host
        );
        audit
            .log(
                "proxmox.vm.snapshot.create",
                &host,
                "dry_run",
                Some(&snapname),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.snapshot.create", &output));
    }

    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;

        let mut params: Vec<(&str, String)> = vec![("snapname", snapname.clone())];
        if let Some(description) = &description {
            params.push(("description", description.clone()));
        }
        if let Some(vmstate) = vmstate {
            params.push(("vmstate", bool_to_proxmox(vmstate).to_string()));
        }

        let param_refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        let path = format!("/nodes/{}/{}/{}/snapshot", node_name, vm_type, vmid);
        let response = client.post(&path, &param_refs).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, task_wait_timeout_secs(&manager, &host))
                .await?;
            Ok(format!(
                "Created snapshot '{}' for {} {} on node '{}'. {}",
                snapname, vm_type, vmid, node_name, result
            ))
        } else {
            Ok(format!(
                "Snapshot '{}' created for {} {} on node '{}'.",
                snapname, vm_type, vmid, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.snapshot.create",
                    &host,
                    "success",
                    Some(&snapname),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.snapshot.create", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.snapshot.create",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn snapshot_rollback(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    snapname: String,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());

    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would rollback {} {} to snapshot '{}' on Proxmox host '{}'.",
            vm_type, vmid, snapname, host
        );
        audit
            .log(
                "proxmox.vm.snapshot.rollback",
                &host,
                "dry_run",
                Some(&snapname),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope(
            "proxmox.vm.snapshot.rollback",
            &output,
        ));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "vmid": vmid,
        "vm_type": vm_type.clone(),
        "snapname": snapname.clone(),
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.snapshot.rollback",
            None,
            &format!(
                "About to ROLLBACK {} {} to snapshot '{}' on Proxmox host '{}'. Current state will be lost.",
                vm_type, vmid, snapname, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.snapshot.rollback",
                &host,
                "confirmation_required",
                Some(&snapname),
            )
            .await
            .ok();
        return Ok(response);
    }

    snapshot_rollback_confirmed(manager, host, node, vmid, vm_type, snapname, audit).await
}

pub async fn snapshot_rollback_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    vmid: u64,
    vm_type: String,
    snapname: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(Some(&vm_type))?;
        let path = format!(
            "/nodes/{}/{}/{}/snapshot/{}/rollback",
            node_name, vm_type, vmid, snapname
        );
        let response = client.post(&path, &[]).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, task_wait_timeout_secs(&manager, &host))
                .await?;
            Ok(format!(
                "Rolled back {} {} to snapshot '{}' on node '{}'. {}",
                vm_type, vmid, snapname, node_name, result
            ))
        } else {
            Ok(format!(
                "Rollback requested for {} {} to snapshot '{}' on node '{}'.",
                vm_type, vmid, snapname, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.snapshot.rollback",
                    &host,
                    "success",
                    Some(&snapname),
                )
                .await
                .ok();
            Ok(wrap_output_envelope(
                "proxmox.vm.snapshot.rollback",
                &output,
            ))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.snapshot.rollback",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn storage_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let storage = client.get(&format!("/nodes/{}/storage", node_name)).await?;

        let storage = storage
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected response format from storage list"))?;

        if storage.is_empty() {
            return Ok(format!("No storage pools found on node '{}'.", node_name));
        }

        let mut lines = vec![format!(
            "Storage on node '{}' (Proxmox host: {})\n\n{:<20}  {:<10}  {:<10}  {:<12}  {:<12}  {:<8}  {}",
            node_name,
            host,
            "STORAGE",
            "TYPE",
            "STATUS",
            "USED",
            "TOTAL",
            "USE%",
            "CONTENT"
        )];

        for pool in storage {
            let name = pool.get("storage").and_then(Value::as_str).unwrap_or("?");
            let pool_type = pool.get("type").and_then(Value::as_str).unwrap_or("?");
            let active = pool.get("active").and_then(Value::as_u64).unwrap_or(0);
            let used = pool.get("used").and_then(Value::as_u64).unwrap_or(0);
            let total = pool.get("total").and_then(Value::as_u64).unwrap_or(0);
            let used_fraction = pool
                .get("used_fraction")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let content = pool.get("content").and_then(Value::as_str).unwrap_or("?");

            lines.push(format!(
                "{:<20}  {:<10}  {:<10}  {:<12}  {:<12}  {:<8.1}%  {}",
                name,
                pool_type,
                if active == 1 { "active" } else { "inactive" },
                format_bytes(used),
                format_bytes(total),
                used_fraction * 100.0,
                content,
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.storage.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.storage.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.storage.list",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

/// Helper function to format VM configuration in a human-readable grouped format
fn format_vm_config(config: &Value, vm_type: &str, vmid: u64) -> String {
    let mut lines = vec![format!(
        "\n=== VM {} Configuration ({}) ===\n",
        vmid,
        vm_type.to_uppercase()
    )];

    // CPU configuration
    let mut cpu_section = vec!["── CPU ──".to_string()];
    if let Some(cores) = config.get("cores").and_then(Value::as_u64) {
        cpu_section.push(format!("  Cores:         {}", cores));
    }
    if let Some(sockets) = config.get("sockets").and_then(Value::as_u64) {
        cpu_section.push(format!("  Sockets:       {}", sockets));
    }
    if let Some(cpu_type) = config.get("cpu").and_then(Value::as_str) {
        cpu_section.push(format!("  CPU type:      {}", cpu_type));
    }
    if let Some(cpulimit) = config.get("cpulimit").and_then(Value::as_f64) {
        cpu_section.push(format!("  CPU limit:     {}", cpulimit));
    }
    if cpu_section.len() > 1 {
        lines.extend(cpu_section);
        lines.push(String::new());
    }

    // Memory configuration
    let mut mem_section = vec!["── Memory ──".to_string()];
    if let Some(memory) = config.get("memory").and_then(Value::as_u64) {
        let memory_gb = memory as f64 / 1024.0;
        mem_section.push(format!(
            "  Memory:        {} MB ({:.1} GB)",
            memory, memory_gb
        ));
    }
    if let Some(balloon) = config.get("balloon").and_then(Value::as_u64) {
        if balloon > 0 {
            let balloon_gb = balloon as f64 / 1024.0;
            mem_section.push(format!(
                "  Balloon (min):  {} MB ({:.1} GB)",
                balloon, balloon_gb
            ));
        }
    }
    if let Some(swap) = config.get("swap").and_then(Value::as_u64) {
        mem_section.push(format!("  Swap:          {} MB", swap));
    }
    if mem_section.len() > 1 {
        lines.extend(mem_section);
        lines.push(String::new());
    }

    // Disk configuration
    let mut disk_section = vec!["── Disks ──".to_string()];
    let disk_keys = vec![
        "scsi0", "scsi1", "scsi2", "scsi3", "virtio0", "virtio1", "ide0", "ide1", "sata0", "sata1",
        "rootfs", "mp0", "mp1", "mp2",
    ];
    let mut has_disks = false;
    for key in &disk_keys {
        if let Some(disk) = config.get(*key).and_then(Value::as_str) {
            disk_section.push(format!("  {}:  {}", key, disk));
            has_disks = true;
        }
    }
    if has_disks {
        lines.extend(disk_section);
        lines.push(String::new());
    }

    // Network configuration
    let mut net_section = vec!["── Network ──".to_string()];
    let net_keys = vec!["net0", "net1", "net2", "net3"];
    let mut has_nets = false;
    for key in &net_keys {
        if let Some(net) = config.get(*key).and_then(Value::as_str) {
            net_section.push(format!("  {}:   {}", key, net));
            has_nets = true;
        }
    }
    if has_nets {
        lines.extend(net_section);
        lines.push(String::new());
    }

    // Cloud-init configuration (QEMU only)
    if vm_type == "qemu" {
        let mut cloud_init_section = vec!["── Cloud-Init ──".to_string()];
        let mut has_cloud_init = false;

        if let Some(ciuser) = config.get("ciuser").and_then(Value::as_str) {
            cloud_init_section.push(format!("  User:          {}", ciuser));
            has_cloud_init = true;
        }
        if config.get("cipassword").is_some() {
            cloud_init_section.push("  Password:      (set)".to_string());
            has_cloud_init = true;
        }
        if let Some(sshkeys) = config.get("sshkeys").and_then(Value::as_str) {
            // Count keys (they're newline-separated when URL-encoded)
            let key_count = sshkeys.matches("%0A").count() + 1;
            cloud_init_section.push(format!("  SSH keys:      ({} keys configured)", key_count));
            has_cloud_init = true;
        }
        if let Some(ipconfig0) = config.get("ipconfig0").and_then(Value::as_str) {
            cloud_init_section.push(format!("  IP config 0:   {}", ipconfig0));
            has_cloud_init = true;
        }
        if let Some(ipconfig1) = config.get("ipconfig1").and_then(Value::as_str) {
            cloud_init_section.push(format!("  IP config 1:   {}", ipconfig1));
            has_cloud_init = true;
        }
        if let Some(nameserver) = config.get("nameserver").and_then(Value::as_str) {
            cloud_init_section.push(format!("  Nameserver:    {}", nameserver));
            has_cloud_init = true;
        }
        if let Some(searchdomain) = config.get("searchdomain").and_then(Value::as_str) {
            cloud_init_section.push(format!("  Search domain: {}", searchdomain));
            has_cloud_init = true;
        }

        if has_cloud_init {
            lines.extend(cloud_init_section);
            lines.push(String::new());
        }
    }

    // Boot configuration
    let mut boot_section = vec!["── Boot ──".to_string()];
    if let Some(boot) = config.get("boot").and_then(Value::as_str) {
        boot_section.push(format!("  Boot order:    {}", boot));
    }
    if let Some(bootdisk) = config.get("bootdisk").and_then(Value::as_str) {
        boot_section.push(format!("  Boot disk:     {}", bootdisk));
    }
    if let Some(ostype) = config.get("ostype").and_then(Value::as_str) {
        boot_section.push(format!("  OS type:       {}", ostype));
    }
    if let Some(machine) = config.get("machine").and_then(Value::as_str) {
        boot_section.push(format!("  Machine:       {}", machine));
    }
    if let Some(bios) = config.get("bios").and_then(Value::as_str) {
        boot_section.push(format!("  BIOS:          {}", bios));
    }
    if boot_section.len() > 1 {
        lines.extend(boot_section);
        lines.push(String::new());
    }

    // Other important configuration
    let mut other_section = vec!["── Other ──".to_string()];
    if let Some(name) = config.get("name").and_then(Value::as_str) {
        other_section.push(format!("  Name:          {}", name));
    }
    if let Some(description) = config.get("description").and_then(Value::as_str) {
        other_section.push(format!("  Description:   {}", description));
    }
    if let Some(onboot) = config.get("onboot").and_then(Value::as_u64) {
        other_section.push(format!(
            "  Start on boot: {}",
            if onboot == 1 { "yes" } else { "no" }
        ));
    }
    if let Some(startup) = config.get("startup").and_then(Value::as_str) {
        other_section.push(format!("  Startup order: {}", startup));
    }
    if other_section.len() > 1 {
        lines.extend(other_section);
        lines.push(String::new());
    }

    lines.join("\n")
}

pub async fn vm_config_get(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type = resolved_vm_type(vm_type.as_deref())?;
        let path = format!("/nodes/{}/{}/{}/config", node_name, vm_type, vmid);
        let config = client.get(&path).await?;

        let formatted = format_vm_config(&config, vm_type, vmid);
        Ok(formatted)
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.config.get",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.config.get", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.config.get",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn vm_config_update(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    vm_type: Option<String>,
    // Core params
    cores: Option<u32>,
    sockets: Option<u32>,
    memory: Option<u32>,
    balloon: Option<u32>,
    cpu_type: Option<String>,
    name: Option<String>,
    description: Option<String>,
    onboot: Option<bool>,
    // Cloud-init
    ciuser: Option<String>,
    cipassword: Option<String>,
    sshkeys: Option<String>,
    ipconfig0: Option<String>,
    ipconfig1: Option<String>,
    nameserver: Option<String>,
    searchdomain: Option<String>,
    // LXC
    swap: Option<u32>,
    cpulimit: Option<f64>,
    unprivileged: Option<bool>,
    // Advanced
    boot: Option<String>,
    ostype: Option<String>,
    machine: Option<String>,
    bios: Option<String>,
    // Net/Disk raw
    net0: Option<String>,
    net1: Option<String>,
    scsi0: Option<String>,
    virtio0: Option<String>,
    // Control
    delete_keys: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let vm_type_str = vm_type.clone().unwrap_or_else(|| "qemu".to_string());
    ensure_vm_type(&vm_type_str)?;

    // Check if at least one param is provided
    if cores.is_none()
        && sockets.is_none()
        && memory.is_none()
        && balloon.is_none()
        && cpu_type.is_none()
        && name.is_none()
        && description.is_none()
        && onboot.is_none()
        && ciuser.is_none()
        && cipassword.is_none()
        && sshkeys.is_none()
        && ipconfig0.is_none()
        && ipconfig1.is_none()
        && nameserver.is_none()
        && searchdomain.is_none()
        && swap.is_none()
        && cpulimit.is_none()
        && unprivileged.is_none()
        && boot.is_none()
        && ostype.is_none()
        && machine.is_none()
        && bios.is_none()
        && net0.is_none()
        && net1.is_none()
        && scsi0.is_none()
        && virtio0.is_none()
        && delete_keys.is_none()
    {
        return Err(anyhow!(
            "No configuration parameters provided. Specify at least one parameter to change."
        ));
    }

    if dry_run.unwrap_or(false) {
        // For dry_run, just show what would be changed
        let mut changes = vec![];
        if cores.is_some() {
            changes.push(format!("cores → {}", cores.unwrap()));
        }
        if memory.is_some() {
            changes.push(format!("memory → {} MB", memory.unwrap()));
        }
        if sockets.is_some() {
            changes.push(format!("sockets → {}", sockets.unwrap()));
        }
        if name.is_some() {
            changes.push(format!("name → {}", name.as_ref().unwrap()));
        }
        if ciuser.is_some() {
            changes.push(format!("ciuser → {}", ciuser.as_ref().unwrap()));
        }
        if ipconfig0.is_some() {
            changes.push(format!("ipconfig0 → {}", ipconfig0.as_ref().unwrap()));
        }

        let output = format!(
            "DRY RUN: Would UPDATE {} {} on Proxmox host '{}' with:\n  {}",
            vm_type_str,
            vmid,
            host,
            changes.join("\n  ")
        );

        audit
            .log(
                "proxmox.vm.config.update",
                &host,
                "dry_run",
                Some(&vmid.to_string()),
            )
            .await
            .ok();

        return Ok(wrap_output_envelope("proxmox.vm.config.update", &output));
    }

    // Build confirmation preview
    let mut preview_lines = vec![format!(
        "\n⚠ Confirm: Update {} {} configuration\n",
        vm_type_str, vmid
    )];
    preview_lines.push("Changes:".to_string());

    if let Some(v) = &cores {
        preview_lines.push(format!("  cores:       → {}", v));
    }
    if let Some(v) = &sockets {
        preview_lines.push(format!("  sockets:     → {}", v));
    }
    if let Some(v) = &memory {
        preview_lines.push(format!("  memory:      → {} MB", v));
    }
    if let Some(v) = &balloon {
        preview_lines.push(format!("  balloon:     → {} MB", v));
    }
    if let Some(v) = &name {
        preview_lines.push(format!("  name:        → {}", v));
    }
    if let Some(v) = &ciuser {
        preview_lines.push(format!("  ciuser:      → {}", v));
    }
    if let Some(v) = &ipconfig0 {
        preview_lines.push(format!("  ipconfig0:   → {}", v));
    }
    if let Some(v) = &ipconfig1 {
        preview_lines.push(format!("  ipconfig1:   → {}", v));
    }

    preview_lines.push("\nUse confirm_operation to apply.".to_string());

    // Store params for confirmation
    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "vmid": vmid,
        "vm_type": vm_type_str.clone(),
        "cores": cores,
        "sockets": sockets,
        "memory": memory,
        "balloon": balloon,
        "cpu_type": cpu_type,
        "name": name,
        "description": description,
        "onboot": onboot,
        "ciuser": ciuser,
        "cipassword": cipassword,
        "sshkeys": sshkeys,
        "ipconfig0": ipconfig0,
        "ipconfig1": ipconfig1,
        "nameserver": nameserver,
        "searchdomain": searchdomain,
        "swap": swap,
        "cpulimit": cpulimit,
        "unprivileged": unprivileged,
        "boot": boot,
        "ostype": ostype,
        "machine": machine,
        "bios": bios,
        "net0": net0,
        "net1": net1,
        "scsi0": scsi0,
        "virtio0": virtio0,
        "delete_keys": delete_keys,
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.config.update",
            None,
            &preview_lines.join("\n"),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.config.update",
                &host,
                "confirmation_required",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(response);
    }

    vm_config_update_confirmed(
        manager,
        host,
        node,
        vmid,
        vm_type_str,
        cores,
        sockets,
        memory,
        balloon,
        cpu_type,
        name,
        description,
        onboot,
        ciuser,
        cipassword,
        sshkeys,
        ipconfig0,
        ipconfig1,
        nameserver,
        searchdomain,
        swap,
        cpulimit,
        unprivileged,
        boot,
        ostype,
        machine,
        bios,
        net0,
        net1,
        scsi0,
        virtio0,
        delete_keys,
        audit,
    )
    .await
}

pub async fn vm_config_update_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    vmid: u64,
    vm_type: String,
    cores: Option<u32>,
    sockets: Option<u32>,
    memory: Option<u32>,
    balloon: Option<u32>,
    cpu_type: Option<String>,
    name: Option<String>,
    description: Option<String>,
    onboot: Option<bool>,
    ciuser: Option<String>,
    cipassword: Option<String>,
    sshkeys: Option<String>,
    ipconfig0: Option<String>,
    ipconfig1: Option<String>,
    nameserver: Option<String>,
    searchdomain: Option<String>,
    swap: Option<u32>,
    cpulimit: Option<f64>,
    unprivileged: Option<bool>,
    boot: Option<String>,
    ostype: Option<String>,
    machine: Option<String>,
    bios: Option<String>,
    net0: Option<String>,
    net1: Option<String>,
    scsi0: Option<String>,
    virtio0: Option<String>,
    delete_keys: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type_resolved = resolved_vm_type(Some(&vm_type))?;
        let path = format!("/nodes/{}/{}/{}/config", node_name, vm_type_resolved, vmid);

        // Build parameter list
        let mut params: Vec<(&str, String)> = Vec::new();
        let cores_str;
        let sockets_str;
        let memory_str;
        let balloon_str;
        let swap_str;
        let cpulimit_str;
        let onboot_str;
        let unprivileged_str;

        if let Some(v) = cores {
            cores_str = v.to_string();
            params.push(("cores", cores_str.clone()));
        }
        if let Some(v) = sockets {
            sockets_str = v.to_string();
            params.push(("sockets", sockets_str.clone()));
        }
        if let Some(v) = memory {
            memory_str = v.to_string();
            params.push(("memory", memory_str.clone()));
        }
        if let Some(v) = balloon {
            balloon_str = v.to_string();
            params.push(("balloon", balloon_str.clone()));
        }
        if let Some(v) = cpu_type {
            params.push(("cpu", v));
        }
        if let Some(v) = name {
            params.push(("name", v));
        }
        if let Some(v) = description {
            params.push(("description", v));
        }
        if let Some(v) = onboot {
            onboot_str = bool_to_proxmox(v).to_string();
            params.push(("onboot", onboot_str.clone()));
        }
        if let Some(v) = ciuser {
            params.push(("ciuser", v));
        }
        if let Some(v) = cipassword {
            params.push(("cipassword", v));
        }
        if let Some(v) = sshkeys {
            // Form-urlencoded will handle encoding automatically
            params.push(("sshkeys", v));
        }
        if let Some(v) = ipconfig0 {
            params.push(("ipconfig0", v));
        }
        if let Some(v) = ipconfig1 {
            params.push(("ipconfig1", v));
        }
        if let Some(v) = nameserver {
            params.push(("nameserver", v));
        }
        if let Some(v) = searchdomain {
            params.push(("searchdomain", v));
        }
        if let Some(v) = swap {
            swap_str = v.to_string();
            params.push(("swap", swap_str.clone()));
        }
        if let Some(v) = cpulimit {
            cpulimit_str = v.to_string();
            params.push(("cpulimit", cpulimit_str.clone()));
        }
        if let Some(v) = unprivileged {
            unprivileged_str = bool_to_proxmox(v).to_string();
            params.push(("unprivileged", unprivileged_str.clone()));
        }
        if let Some(v) = boot {
            params.push(("boot", v));
        }
        if let Some(v) = ostype {
            params.push(("ostype", v));
        }
        if let Some(v) = machine {
            params.push(("machine", v));
        }
        if let Some(v) = bios {
            params.push(("bios", v));
        }
        if let Some(v) = net0 {
            params.push(("net0", v));
        }
        if let Some(v) = net1 {
            params.push(("net1", v));
        }
        if let Some(v) = scsi0 {
            params.push(("scsi0", v));
        }
        if let Some(v) = virtio0 {
            params.push(("virtio0", v));
        }
        if let Some(v) = delete_keys {
            params.push(("delete", v));
        }

        // Convert to &[(&str, &str)] for the API call
        let param_refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let _response = client.put(&path, &param_refs).await?;

        Ok(format!(
            "Configuration updated successfully for {} {} on node '{}'.",
            vm_type_resolved, vmid, node_name
        ))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.config.update",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.config.update", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.config.update",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn backup_create(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    vmid: u64,
    storage: String,
    mode: Option<String>,
    compress: Option<String>,
    notes: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;

    let mode = mode.unwrap_or_else(|| "snapshot".to_string());
    match mode.as_str() {
        "snapshot" | "suspend" | "stop" => {}
        other => {
            return Err(anyhow!(
                "mode must be 'snapshot', 'suspend', or 'stop'. Got '{}'",
                other
            ));
        }
    }
    if let Some(ref c) = compress {
        match c.as_str() {
            "zstd" | "lzo" | "gzip" | "0" => {}
            other => {
                return Err(anyhow!(
                    "compress must be 'zstd', 'lzo', 'gzip', or '0'. Got '{}'",
                    other
                ));
            }
        }
    }

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would create vzdump backup of VM {} to storage '{}' on Proxmox host '{}'.\n\
             Mode: {}\nCompress: {}",
            vmid,
            storage,
            host,
            mode,
            compress.as_deref().unwrap_or("(default)"),
        );
        audit
            .log(
                "proxmox.vm.backup.create",
                &host,
                "dry_run",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.backup.create", &output));
    }

    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;

        let mut params: Vec<(&str, String)> = vec![
            ("vmid", vmid.to_string()),
            ("storage", storage.clone()),
            ("mode", mode.clone()),
        ];
        if let Some(ref c) = compress {
            params.push(("compress", c.clone()));
        }
        if let Some(ref n) = notes {
            params.push(("notes-template", n.clone()));
        }

        let param_refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();

        let path = format!("/nodes/{}/vzdump", node_name);
        let response = client.post(&path, &param_refs).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, task_wait_timeout_secs(&manager, &host))
                .await?;
            Ok(format!(
                "Backup created for VM {} on storage '{}' (node '{}'). {}",
                vmid, storage, node_name, result
            ))
        } else {
            Ok(format!(
                "Backup requested for VM {} on storage '{}' (node '{}').",
                vmid, storage, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.backup.create",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.backup.create", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.backup.create",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn backup_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    storage: String,
    vmid: Option<u64>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let path = format!("/nodes/{}/storage/{}/content?content=backup", node_name, storage);
        let content = client.get(&path).await?;

        let items = content
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected response format from storage content list"))?;

        // Filter by vmid if specified
        let filtered: Vec<&Value> = items
            .iter()
            .filter(|item| {
                if let Some(filter_vmid) = vmid {
                    item.get("vmid").and_then(Value::as_u64) == Some(filter_vmid)
                } else {
                    true
                }
            })
            .collect();

        if filtered.is_empty() {
            let filter_msg = vmid
                .map(|id| format!(" for VM {}", id))
                .unwrap_or_default();
            return Ok(format!(
                "No backups found{} on storage '{}' (node '{}').",
                filter_msg, storage, node_name
            ));
        }

        let mut lines = vec![format!(
            "Backups on storage '{}' (node '{}', Proxmox host: {})\n\n{:<60}  {:<8}  {:<12}  {:<10}  {}",
            storage, node_name, host, "VOLUME ID", "VMID", "SIZE", "FORMAT", "NOTES"
        )];

        for item in &filtered {
            let volid = item.get("volid").and_then(Value::as_str).unwrap_or("?");
            let item_vmid = item.get("vmid").and_then(Value::as_u64).unwrap_or(0);
            let size = item.get("size").and_then(Value::as_u64).unwrap_or(0);
            let fmt = item.get("format").and_then(Value::as_str).unwrap_or("?");
            let notes = item.get("notes").and_then(Value::as_str).unwrap_or("");

            lines.push(format!(
                "{:<60}  {:<8}  {:<12}  {:<10}  {}",
                volid,
                item_vmid,
                format_bytes(size),
                fmt,
                truncate_cell(notes, 40),
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.vm.backup.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.backup.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.backup.list",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn backup_restore(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    archive: String,
    vmid: u64,
    storage: Option<String>,
    vm_type: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let vm_type = vm_type.unwrap_or_else(|| "qemu".to_string());
    ensure_vm_type(&vm_type)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would restore {} {} from archive '{}' on Proxmox host '{}'.\n\
             Target storage: {}",
            vm_type,
            vmid,
            archive,
            host,
            storage.as_deref().unwrap_or("(default)"),
        );
        audit
            .log(
                "proxmox.vm.backup.restore",
                &host,
                "dry_run",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.vm.backup.restore", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "archive": archive.clone(),
        "vmid": vmid,
        "storage": storage.clone(),
        "vm_type": vm_type.clone(),
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.vm.backup.restore",
            None,
            &format!(
                "About to RESTORE {} {} from archive '{}' on Proxmox host '{}'. If VMID {} already exists, the operation will fail.",
                vm_type, vmid, archive, host, vmid
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.vm.backup.restore",
                &host,
                "confirmation_required",
                Some(&vmid.to_string()),
            )
            .await
            .ok();
        return Ok(response);
    }

    backup_restore_confirmed(manager, host, node, archive, vmid, storage, vm_type, audit).await
}

#[allow(clippy::too_many_arguments)]
pub async fn backup_restore_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    archive: String,
    vmid: u64,
    storage: Option<String>,
    vm_type: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let vm_type_resolved = resolved_vm_type(Some(&vm_type))?;

        let mut params: Vec<(&str, String)> =
            vec![("vmid", vmid.to_string()), ("archive", archive.clone())];
        if let Some(ref s) = storage {
            params.push(("storage", s.clone()));
        }

        let param_refs: Vec<(&str, &str)> = params
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();

        let path = format!("/nodes/{}/{}", node_name, vm_type_resolved);
        let response = client.post(&path, &param_refs).await?;

        if let Some(upid) = response.as_str() {
            let result = client
                .wait_for_task(&node_name, upid, task_wait_timeout_secs(&manager, &host))
                .await?;
            Ok(format!(
                "Restored {} {} from '{}' on node '{}'. {}",
                vm_type_resolved, vmid, archive, node_name, result
            ))
        } else {
            Ok(format!(
                "Restore requested for {} {} from '{}' on node '{}'.",
                vm_type_resolved, vmid, archive, node_name
            ))
        }
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log(
                    "proxmox.vm.backup.restore",
                    &host,
                    "success",
                    Some(&vmid.to_string()),
                )
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.vm.backup.restore", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.vm.backup.restore",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn network_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let networks = client.get(&format!("/nodes/{}/network", node_name)).await?;

        let networks = networks
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected response format from network list"))?;

        if networks.is_empty() {
            return Ok(format!("No network interfaces found on node '{}'.", node_name));
        }

        let mut lines = vec![format!(
            "Network interfaces on node '{}' (Proxmox host: {})\n\n{:<16}  {:<10}  {:<18}  {:<10}  {:<8}  {}",
            node_name, host, "IFACE", "TYPE", "ADDRESS", "ACTIVE", "AUTO", "BRIDGE PORTS"
        )];

        for iface in networks {
            let name = iface.get("iface").and_then(Value::as_str).unwrap_or("?");
            let iface_type = iface.get("type").and_then(Value::as_str).unwrap_or("?");
            let address = iface.get("address").and_then(Value::as_str).unwrap_or("-");
            let cidr = iface.get("cidr").and_then(Value::as_str);
            let active = iface.get("active").and_then(Value::as_u64).unwrap_or(0);
            let autostart = iface.get("autostart").and_then(Value::as_u64).unwrap_or(0);
            let bridge_ports = iface
                .get("bridge_ports")
                .and_then(Value::as_str)
                .unwrap_or("-");

            lines.push(format!(
                "{:<16}  {:<10}  {:<18}  {:<10}  {:<8}  {}",
                name,
                iface_type,
                cidr.unwrap_or(address),
                if active == 1 { "yes" } else { "no" },
                if autostart == 1 { "yes" } else { "no" },
                bridge_ports,
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.network.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.network.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.network.list",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub(crate) fn default_proxmox_host(manager: &ConnectionManager) -> Result<String> {
    let hosts = &manager.config().proxmox.hosts;

    match hosts.len() {
        0 => Err(anyhow!("No Proxmox hosts are configured.")),
        1 => hosts
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| anyhow!("No Proxmox hosts are configured.")),
        _ => {
            let mut host_names = hosts.keys().cloned().collect::<Vec<_>>();
            host_names.sort();
            Err(anyhow!(
                "Multiple Proxmox hosts are configured ({}). Pass 'host' explicitly.",
                host_names.join(", ")
            ))
        }
    }
}

fn task_wait_timeout_secs(manager: &ConnectionManager, host: &str) -> u64 {
    manager
        .config()
        .proxmox
        .hosts
        .get(host)
        .map(|host| host.task_wait_timeout_secs)
        .unwrap_or(DEFAULT_TASK_WAIT_TIMEOUT_SECS)
}

fn auto_vmid_retry_policy(manager: &ConnectionManager, host: &str) -> AutoVmidRetryPolicy {
    manager
        .config()
        .proxmox
        .hosts
        .get(host)
        .map(|host| AutoVmidRetryPolicy {
            attempts: host.next_vmid_retry_attempts,
            backoff_ms: host.next_vmid_retry_backoff_ms,
        })
        .unwrap_or(AutoVmidRetryPolicy {
            attempts: 3,
            backoff_ms: 250,
        })
}

fn ensure_vm_type(vm_type: &str) -> Result<()> {
    match vm_type {
        "qemu" | "lxc" => Ok(()),
        other => Err(anyhow!("vm_type must be 'qemu' or 'lxc'. Got '{}'", other)),
    }
}

fn resolved_vm_type(vm_type: Option<&str>) -> Result<&str> {
    let vm_type = vm_type.unwrap_or("qemu");
    ensure_vm_type(vm_type)?;
    Ok(vm_type)
}

fn validate_vm_create_platform_inputs(
    vm_type: &str,
    os_type: Option<&str>,
    template: Option<&str>,
) -> Result<()> {
    match vm_type {
        "qemu" if template.is_some() => {
            Err(anyhow!("template is only valid when vm_type is 'lxc'"))
        }
        "lxc" if os_type.is_some() => Err(anyhow!("os_type is only valid when vm_type is 'qemu'")),
        _ => Ok(()),
    }
}

async fn next_vmid(client: &ProxmoxClient) -> Result<u64> {
    let next = client.get("/cluster/nextid").await?;
    next.as_str()
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| next.as_u64())
        .ok_or_else(|| anyhow!("Failed to get next available VMID"))
}

fn is_duplicate_vmid_conflict(error: &anyhow::Error, vmid: u64) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    let vmid_text = vmid.to_string();
    let mentions_conflict = message.contains("already exists")
        || message.contains("already used")
        || message.contains("already in use")
        || message.contains("duplicate");
    let mentions_vmid = message.contains(&format!("vm {}", vmid))
        || message.contains(&format!("ct {}", vmid))
        || (message.contains("vmid") && message.contains(&vmid_text));

    mentions_conflict && mentions_vmid
}

fn exhausted_auto_vmid_error(
    tool_name: &str,
    field_name: &str,
    retry_policy: AutoVmidRetryPolicy,
    last_error: anyhow::Error,
) -> anyhow::Error {
    anyhow!(
        "{} failed after {} auto-VMID conflict retries ({}ms backoff). Pass an explicit {} or increase next_vmid_retry_attempts/next_vmid_retry_backoff_ms. Last error: {}",
        tool_name,
        retry_policy.attempts,
        retry_policy.backoff_ms,
        field_name,
        last_error,
    )
}

fn bool_to_proxmox(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn percent(used: u64, total: u64) -> f64 {
    if total > 0 {
        (used as f64 / total as f64) * 100.0
    } else {
        0.0
    }
}

fn format_loadavg(loadavg: Option<&Value>) -> String {
    match loadavg.and_then(Value::as_array) {
        Some(values) if !values.is_empty() => values
            .iter()
            .map(|value| match value {
                Value::String(value) => value.clone(),
                Value::Number(value) => value.to_string(),
                _ => "?".to_string(),
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => "?".to_string(),
    }
}

fn truncate_cell(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }

    value.chars().take(max_chars).collect()
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;

    if days > 0 {
        format!("{}d {}h {}m", days, hours, minutes)
    } else if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn network_create(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    node: Option<String>,
    iface: String,
    iface_type: String,
    address: Option<String>,
    netmask: Option<String>,
    gateway: Option<String>,
    address6: Option<String>,
    netmask6: Option<String>,
    gateway6: Option<String>,
    bridge_ports: Option<String>,
    bond_mode: Option<String>,
    vlan_id: Option<u32>,
    vlan_raw_device: Option<String>,
    autostart: Option<bool>,
    comments: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would create network interface '{}' (type: {}) on Proxmox host '{}'.\n\
             Note: Network changes are staged — call proxmox.network.apply to make them live.",
            iface, iface_type, host
        );
        audit
            .log("proxmox.network.create", &host, "dry_run", Some(&iface))
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.network.create", &output));
    }

    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let path = format!("/nodes/{}/network", node_name);

        let mut params: Vec<(&str, String)> =
            vec![("iface", iface.clone()), ("type", iface_type.clone())];
        if let Some(ref v) = address {
            params.push(("address", v.clone()));
        }
        if let Some(ref v) = netmask {
            params.push(("netmask", v.clone()));
        }
        if let Some(ref v) = gateway {
            params.push(("gateway", v.clone()));
        }
        if let Some(ref v) = address6 {
            params.push(("address6", v.clone()));
        }
        if let Some(ref v) = netmask6 {
            params.push(("netmask6", v.clone()));
        }
        if let Some(ref v) = gateway6 {
            params.push(("gateway6", v.clone()));
        }
        if let Some(ref v) = bridge_ports {
            params.push(("bridge_ports", v.clone()));
        }
        if let Some(ref v) = bond_mode {
            params.push(("bond_mode", v.clone()));
        }
        if let Some(v) = vlan_id {
            params.push(("vlan-id", v.to_string()));
        }
        if let Some(ref v) = vlan_raw_device {
            params.push(("vlan-raw-device", v.clone()));
        }
        if let Some(v) = autostart {
            params.push(("autostart", if v { "1" } else { "0" }.to_string()));
        }
        if let Some(ref v) = comments {
            params.push(("comments", v.clone()));
        }

        let params_ref: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        client.post(&path, &params_ref).await?;

        Ok(format!(
            "Created network interface '{}' (type: {}) on node '{}' (host: '{}').\n\
             Note: This change is STAGED. Call proxmox.network.apply to make it live.",
            iface, iface_type, node_name, host
        ))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.network.create", &host, "success", Some(&iface))
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.network.create", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.network.create",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn network_update(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    iface: String,
    address: Option<String>,
    netmask: Option<String>,
    gateway: Option<String>,
    address6: Option<String>,
    netmask6: Option<String>,
    gateway6: Option<String>,
    bridge_ports: Option<String>,
    bond_mode: Option<String>,
    vlan_id: Option<u32>,
    vlan_raw_device: Option<String>,
    autostart: Option<bool>,
    comments: Option<String>,
    iface_type: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would update network interface '{}' on Proxmox host '{}'.\n\
             Note: Network changes are staged — call proxmox.network.apply to make them live.",
            iface, host
        );
        audit
            .log("proxmox.network.update", &host, "dry_run", Some(&iface))
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.network.update", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "iface": iface.clone(),
        "address": address,
        "netmask": netmask,
        "gateway": gateway,
        "address6": address6,
        "netmask6": netmask6,
        "gateway6": gateway6,
        "bridge_ports": bridge_ports,
        "bond_mode": bond_mode,
        "vlan_id": vlan_id,
        "vlan_raw_device": vlan_raw_device,
        "autostart": autostart,
        "comments": comments,
        "type": iface_type,
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.network.update",
            None,
            &format!(
                "About to MODIFY network interface '{}' on Proxmox host '{}'. \
                 Changing network configuration can break connectivity.",
                iface, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.network.update",
                &host,
                "confirmation_required",
                Some(&iface),
            )
            .await
            .ok();
        return Ok(response);
    }

    network_update_confirmed(
        manager,
        host,
        node,
        iface,
        address,
        netmask,
        gateway,
        address6,
        netmask6,
        gateway6,
        bridge_ports,
        bond_mode,
        vlan_id,
        vlan_raw_device,
        autostart,
        comments,
        iface_type,
        audit,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn network_update_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    iface: String,
    address: Option<String>,
    netmask: Option<String>,
    gateway: Option<String>,
    address6: Option<String>,
    netmask6: Option<String>,
    gateway6: Option<String>,
    bridge_ports: Option<String>,
    bond_mode: Option<String>,
    vlan_id: Option<u32>,
    vlan_raw_device: Option<String>,
    autostart: Option<bool>,
    comments: Option<String>,
    iface_type: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let path = format!("/nodes/{}/network/{}", node_name, iface);

        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(ref v) = iface_type {
            params.push(("type", v.clone()));
        }
        if let Some(ref v) = address {
            params.push(("address", v.clone()));
        }
        if let Some(ref v) = netmask {
            params.push(("netmask", v.clone()));
        }
        if let Some(ref v) = gateway {
            params.push(("gateway", v.clone()));
        }
        if let Some(ref v) = address6 {
            params.push(("address6", v.clone()));
        }
        if let Some(ref v) = netmask6 {
            params.push(("netmask6", v.clone()));
        }
        if let Some(ref v) = gateway6 {
            params.push(("gateway6", v.clone()));
        }
        if let Some(ref v) = bridge_ports {
            params.push(("bridge_ports", v.clone()));
        }
        if let Some(ref v) = bond_mode {
            params.push(("bond_mode", v.clone()));
        }
        if let Some(v) = vlan_id {
            params.push(("vlan-id", v.to_string()));
        }
        if let Some(ref v) = vlan_raw_device {
            params.push(("vlan-raw-device", v.clone()));
        }
        if let Some(v) = autostart {
            params.push(("autostart", if v { "1" } else { "0" }.to_string()));
        }
        if let Some(ref v) = comments {
            params.push(("comments", v.clone()));
        }

        let params_ref: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        client.put(&path, &params_ref).await?;

        Ok(format!(
            "Updated network interface '{}' on node '{}' (host: '{}').\n\
             Note: This change is STAGED. Call proxmox.network.apply to make it live.",
            iface, node_name, host
        ))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.network.update", &host, "success", Some(&iface))
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.network.update", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.network.update",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn network_delete(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    iface: String,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would delete network interface '{}' on Proxmox host '{}'.\n\
             Note: Network changes are staged — call proxmox.network.apply to make them live.",
            iface, host
        );
        audit
            .log("proxmox.network.delete", &host, "dry_run", Some(&iface))
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.network.delete", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
        "iface": iface.clone(),
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.network.delete",
            None,
            &format!(
                "About to DELETE network interface '{}' on Proxmox host '{}'. \
                 Removing a bridge can isolate VMs that depend on it.",
                iface, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.network.delete",
                &host,
                "confirmation_required",
                Some(&iface),
            )
            .await
            .ok();
        return Ok(response);
    }

    network_delete_confirmed(manager, host, node, iface, audit).await
}

pub async fn network_delete_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    iface: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let path = format!("/nodes/{}/network/{}", node_name, iface);
        client.delete(&path).await?;

        Ok(format!(
            "Deleted network interface '{}' on node '{}' (host: '{}').\n\
             Note: This change is STAGED. Call proxmox.network.apply to make it live.",
            iface, node_name, host
        ))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.network.delete", &host, "success", Some(&iface))
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.network.delete", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.network.delete",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn network_apply(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    node: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.map_or_else(|| default_proxmox_host(&manager), Ok)?;

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would apply all pending network configuration changes on Proxmox host '{}'.\n\
             WARNING: Applying bad network config can isolate the host.",
            host
        );
        audit
            .log("proxmox.network.apply", &host, "dry_run", None)
            .await
            .ok();
        return Ok(wrap_output_envelope("proxmox.network.apply", &output));
    }

    let params_json = serde_json::json!({
        "host": host.clone(),
        "node": node.clone(),
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "proxmox.network.apply",
            None,
            &format!(
                "About to APPLY all pending network changes on Proxmox host '{}'. \
                 Applying bad config to the management bridge (vmbr0) can isolate the host.",
                host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "proxmox.network.apply",
                &host,
                "confirmation_required",
                None,
            )
            .await
            .ok();
        return Ok(response);
    }

    network_apply_confirmed(manager, host, node, audit).await
}

pub async fn network_apply_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    node: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let client = manager.get_proxmox(&host)?;
        let node_name = client.resolve_node(node.as_deref()).await?;
        let path = format!("/nodes/{}/network", node_name);
        client.put(&path, &[]).await?;

        Ok(format!(
            "Applied pending network configuration changes on node '{}' (host: '{}').\n\
             Network changes are now live.",
            node_name, host
        ))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("proxmox.network.apply", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("proxmox.network.apply", &output))
        }
        Err(error) => {
            audit
                .log(
                    "proxmox.network.apply",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AutoVmidRetryPolicy, exhausted_auto_vmid_error, is_duplicate_vmid_conflict,
        validate_vm_create_platform_inputs,
    };

    #[test]
    fn qemu_accepts_os_type_without_template() {
        assert!(validate_vm_create_platform_inputs("qemu", Some("l26"), None).is_ok());
    }

    #[test]
    fn lxc_accepts_template_without_os_type() {
        assert!(
            validate_vm_create_platform_inputs(
                "lxc",
                None,
                Some("local:vztmpl/ubuntu-24.04-standard_24.04-1_amd64.tar.zst")
            )
            .is_ok()
        );
    }

    #[test]
    fn qemu_rejects_lxc_template_field() {
        let error =
            validate_vm_create_platform_inputs("qemu", Some("l26"), Some("template")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("template is only valid when vm_type is 'lxc'")
        );
    }

    #[test]
    fn lxc_rejects_qemu_os_type_field() {
        let error =
            validate_vm_create_platform_inputs("lxc", Some("l26"), Some("template")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("os_type is only valid when vm_type is 'qemu'")
        );
    }

    #[test]
    fn detects_duplicate_vmid_conflict_error() {
        let error = anyhow::anyhow!(
            "Proxmox API error (HTTP 500): {{\"errors\":{{\"vmid\":\"VM 123 already exists\"}}}}"
        );

        assert!(is_duplicate_vmid_conflict(&error, 123));
    }

    #[test]
    fn ignores_non_vmid_conflict_error() {
        let error = anyhow::anyhow!(
            "Proxmox API error (HTTP 500): {{\"errors\":{{\"name\":\"guest name already exists\"}}}}"
        );

        assert!(!is_duplicate_vmid_conflict(&error, 123));
    }

    #[test]
    fn exhausted_auto_vmid_error_mentions_config_and_field() {
        let error = exhausted_auto_vmid_error(
            "proxmox.vm.create",
            "vmid",
            AutoVmidRetryPolicy {
                attempts: 3,
                backoff_ms: 250,
            },
            anyhow::anyhow!("VM 123 already exists"),
        );

        let message = error.to_string();
        assert!(message.contains("proxmox.vm.create failed after 3 auto-VMID conflict retries"));
        assert!(message.contains("Pass an explicit vmid"));
        assert!(message.contains("next_vmid_retry_attempts/next_vmid_retry_backoff_ms"));
    }
}
