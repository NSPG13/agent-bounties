(function installAgentBounties(documentObject, windowObject) {
  "use strict";

  const body = documentObject.body;
  const platformSlug = body?.dataset.installPlatform;
  const manifestUrl = body?.dataset.installManifest;
  const detail = documentObject.querySelector("[data-install-detail]");
  const status = documentObject.querySelector("[data-install-status]");

  if (!body || !platformSlug || !manifestUrl || !detail || !status) return;

  function element(tag, className, text) {
    const node = documentObject.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function copyText(value, button) {
    const complete = () => {
      const original = button.textContent;
      button.textContent = "Copied";
      windowObject.setTimeout(() => {
        button.textContent = original;
      }, 1600);
    };

    if (windowObject.navigator.clipboard?.writeText) {
      windowObject.navigator.clipboard.writeText(value).then(complete).catch(() => fallbackCopy(value, complete));
      return;
    }
    fallbackCopy(value, complete);
  }

  function fallbackCopy(value, complete) {
    const field = element("textarea");
    field.value = value;
    field.setAttribute("readonly", "");
    field.style.position = "fixed";
    field.style.opacity = "0";
    documentObject.body.appendChild(field);
    field.select();
    const copied = documentObject.execCommand("copy");
    field.remove();
    if (copied) complete();
  }

  function copyBlock(label, value) {
    const block = element("div", "copy-block");
    block.appendChild(element("strong", "", label));
    const pre = element("pre");
    pre.appendChild(element("code", "", value));
    block.appendChild(pre);
    const button = element("button", "", "Copy");
    button.type = "button";
    button.addEventListener("click", () => copyText(value, button));
    block.appendChild(button);
    return block;
  }

  function vscodeInstallUrl(platform) {
    const config = {
      name: "agent-bounties",
      type: "http",
      url: platform.mcp_url,
    };
    return `vscode:mcp/install?${encodeURIComponent(JSON.stringify(config))}`;
  }

  function cursorInstallUrl(platform) {
    const config = windowObject.btoa(JSON.stringify({ type: "http", url: platform.mcp_url }));
    return `https://cursor.com/link/mcp/install?name=agent-bounties&config=${encodeURIComponent(config)}`;
  }

  function linkButton(label, href) {
    const link = element("a", "button button-primary", label);
    link.href = href;
    return link;
  }

  function renderAction(action, platform) {
    if (action.kind === "copy") return copyBlock(action.label, action.value);
    if (action.kind === "vscode_install") return linkButton(action.label, vscodeInstallUrl(platform));
    if (action.kind === "cursor_install") return linkButton(action.label, cursorInstallUrl(platform));
    throw new Error(`Unsupported install action: ${action.kind}`);
  }

  function renderPlatform(platform, manifest) {
    status.textContent = platform.status;
    const grid = element("div", "install-grid");

    const endpointPanel = element("section", "install-panel install-panel-wide");
    endpointPanel.appendChild(element("h2", "", "Attributed MCP endpoint"));
    endpointPanel.appendChild(element("p", "", "This URL reaches the canonical Agent Bounties service while preserving the installation rail for aggregate outcome measurement."));
    const endpointRow = element("div", "endpoint-row");
    endpointRow.appendChild(element("code", "", platform.mcp_url));
    const endpointButton = element("button", "", "Copy endpoint");
    endpointButton.type = "button";
    endpointButton.addEventListener("click", () => copyText(platform.mcp_url, endpointButton));
    endpointRow.appendChild(endpointButton);
    endpointPanel.appendChild(endpointRow);
    grid.appendChild(endpointPanel);

    const stepsPanel = element("section", "install-panel");
    stepsPanel.appendChild(element("h2", "", "Connect safely"));
    const steps = element("ol");
    platform.steps.forEach((step) => steps.appendChild(element("li", "", step)));
    stepsPanel.appendChild(steps);
    grid.appendChild(stepsPanel);

    const actionsPanel = element("section", "install-panel");
    actionsPanel.appendChild(element("h2", "", "Install"));
    const actions = element("div", "action-stack");
    platform.actions.forEach((action) => actions.appendChild(renderAction(action, platform)));
    actionsPanel.appendChild(actions);
    grid.appendChild(actionsPanel);

    const promptPanel = element("section", "install-panel install-panel-wide");
    promptPanel.appendChild(element("h2", "", "First useful request"));
    promptPanel.appendChild(copyBlock("Copy this prompt", platform.first_prompt));
    grid.appendChild(promptPanel);

    const docsPanel = element("section", "install-panel install-panel-wide");
    docsPanel.appendChild(element("h2", "", "Review before connecting"));
    const docs = element("ul", "documentation-list");
    platform.documentation.forEach((entry) => {
      const item = element("li");
      const link = element("a", "", entry.label);
      link.href = entry.url;
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      item.appendChild(link);
      docs.appendChild(item);
    });
    docsPanel.appendChild(docs);
    grid.appendChild(docsPanel);

    detail.replaceChildren(grid);
    const boundary = documentObject.querySelector("[data-payment-boundary]");
    if (boundary) boundary.textContent = manifest.payment_boundary;
  }

  function renderHub(platforms) {
    status.textContent = "Choose the environment where your agent already works.";
    const grid = element("div", "platform-grid");
    platforms.forEach((platform) => {
      const card = element("article", "platform-card");
      card.appendChild(element("div", "platform-status", platform.status));
      card.appendChild(element("h2", "", platform.name));
      card.appendChild(element("p", "", platform.summary));
      const link = element("a", "", `Install for ${platform.name} →`);
      link.href = `./${platform.slug}/`;
      card.appendChild(link);
      grid.appendChild(card);
    });
    detail.replaceChildren(grid);
  }

  windowObject.fetch(manifestUrl, { headers: { accept: "application/json" } })
    .then((response) => {
      if (!response.ok) throw new Error(`Install manifest returned ${response.status}`);
      return response.json();
    })
    .then((manifest) => {
      if (manifest.schema_version !== "agent-bounties/install-platforms-v1") {
        throw new Error("Unsupported install manifest");
      }
      if (platformSlug === "all") {
        renderHub(manifest.platforms);
        return;
      }
      const entries = manifest.platforms.concat(manifest.paid_vendors || []);
      const platform = entries.find((candidate) => candidate.slug === platformSlug);
      if (!platform) throw new Error(`Unknown install platform: ${platformSlug}`);
      renderPlatform(platform, manifest);
    })
    .catch((error) => {
      status.textContent = "The interactive installer could not load. Copy the attributed endpoint shown below or use the source documentation.";
      status.classList.add("install-error");
      detail.dataset.installError = error.message;
      detail.replaceChildren(copyBlock("Copy endpoint", body.dataset.installFallbackEndpoint));
    });
})(document, window);
