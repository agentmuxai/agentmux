// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/app/store/global";
import { isWindows } from "@/util/platformutil";
import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import { PROVIDERS, type ProviderDefinition } from "../providers";

interface SetupWizardProps {
    onSetupComplete: (config: ProviderConfig) => void;
}

type WizardStep = "detect" | "select" | "confirm";

const PROVIDER_ICONS: Record<string, string> = {
    claude: "sparkles",
    gemini: "diamond",
    codex: "robot",
};

export const SetupWizard = ({ onSetupComplete }: SetupWizardProps): JSX.Element => {
    const [step, setStep] = createSignal<WizardStep>("detect");
    const [detectionResults, setDetectionResults] = createSignal<CliDetectionResult[]>([]);
    const [detecting, setDetecting] = createSignal(false);
    const [selectedProvider, setSelectedProvider] = createSignal<string>("");
    const [error, setError] = createSignal<string | null>(null);

    const runDetection = async () => {
        setDetecting(true);
        setError(null);
        try {
            const results = await getApi().detectInstalledClis();
            setDetectionResults(results);

            // Pre-select first installed provider
            const firstInstalled = results.find((r) => r.installed);
            if (firstInstalled) {
                setSelectedProvider(firstInstalled.provider);
            }
        } catch (err) {
            setError(`Detection failed: ${String(err)}`);
        } finally {
            setDetecting(false);
        }
    };

    // Run detection on mount
    onMount(() => {
        void runDetection();
    });

    const handleConfirm = async () => {
        if (!selectedProvider()) return;

        const providerDef = PROVIDERS[selectedProvider()];
        if (!providerDef) return;

        const config: ProviderConfig = {
            default_provider: selectedProvider(),
            providers: {
                [selectedProvider()]: {
                    cli_path: detectionResults().find((r) => r.provider === selectedProvider())?.path ?? null,
                    auth_token: null,
                    auth_status: "none",
                    output_format: providerDef.outputFormat,
                    extra_args: [],
                },
            },
            setup_complete: true,
        };

        try {
            await getApi().saveProviderConfig(config);
            onSetupComplete(config);
        } catch (err) {
            setError(`Failed to save config: ${String(err)}`);
        }
    };

    const installedCount = () => detectionResults().filter((r) => r.installed).length;
    const hasInstalled = () => installedCount() > 0;

    return (
        <div class="setup-wizard">
            <div class="setup-wizard-header">
                <div class="setup-wizard-title">Agent Setup</div>
                <div class="setup-wizard-subtitle">Configure your AI coding assistant</div>
                <div class="setup-wizard-steps">
                    <StepIndicator step="detect" label="Detect" currentStep={step()} />
                    <div class="setup-wizard-step-connector" />
                    <StepIndicator step="select" label="Select" currentStep={step()} />
                    <div class="setup-wizard-step-connector" />
                    <StepIndicator step="confirm" label="Start" currentStep={step()} />
                </div>
            </div>

            <div class="setup-wizard-content">
                <Show when={error()}>
                    <div class="setup-wizard-error">
                        <span class="setup-wizard-error-icon">!</span>
                        {error()}
                    </div>
                </Show>

                <Show when={step() === "detect"}>
                    <DetectStep
                        results={detectionResults()}
                        detecting={detecting()}
                        onRefresh={runDetection}
                        onNext={() => setStep("select")}
                        hasInstalled={hasInstalled()}
                    />
                </Show>

                <Show when={step() === "select"}>
                    <SelectStep
                        results={detectionResults()}
                        selectedProvider={selectedProvider()}
                        onSelect={setSelectedProvider}
                        onBack={() => setStep("detect")}
                        onNext={() => setStep("confirm")}
                    />
                </Show>

                <Show when={step() === "confirm"}>
                    <ConfirmStep
                        selectedProvider={selectedProvider()}
                        detectionResult={detectionResults().find((r) => r.provider === selectedProvider())}
                        onBack={() => setStep("select")}
                        onConfirm={handleConfirm}
                    />
                </Show>
            </div>
        </div>
    );
};

SetupWizard.displayName = "SetupWizard";

// --- Step Indicator ---

const StepIndicator = ({
    step,
    label,
    currentStep,
}: {
    step: WizardStep;
    label: string;
    currentStep: WizardStep;
}): JSX.Element => {
    const steps: WizardStep[] = ["detect", "select", "confirm"];
    const current = steps.indexOf(currentStep);
    const index = steps.indexOf(step);
    const isActive = index === current;
    const isDone = index < current;

    return (
        <div
            class={`setup-wizard-step-indicator ${isActive ? "active" : ""} ${isDone ? "done" : ""}`}
        >
            <div class="setup-wizard-step-dot">
                {isDone ? "\u2713" : index + 1}
            </div>
            <div class="setup-wizard-step-label">{label}</div>
        </div>
    );
};

StepIndicator.displayName = "StepIndicator";

// --- Detect Step ---

const DetectStep = ({
    results,
    detecting,
    onRefresh,
    onNext,
    hasInstalled,
}: {
    results: CliDetectionResult[];
    detecting: boolean;
    onRefresh: () => void;
    onNext: () => void;
    hasInstalled: boolean;
}): JSX.Element => {
    return (
        <div class="setup-wizard-detect">
            <div class="setup-wizard-section-title">Detected CLI Tools</div>
            <div class="setup-wizard-section-desc">
                Scanning for installed AI coding assistants...
            </div>

            <div class="setup-wizard-cli-list">
                <For each={results}>
                    {(result) => <CliResultCard result={result} />}
                </For>
                <Show when={detecting && results.length === 0}>
                    <div class="setup-wizard-detecting">Scanning...</div>
                </Show>
            </div>

            <div class="setup-wizard-actions">
                <button
                    class="setup-wizard-btn secondary"
                    onClick={onRefresh}
                    disabled={detecting}
                >
                    {detecting ? "Scanning..." : "Refresh"}
                </button>
                <button
                    class="setup-wizard-btn primary"
                    onClick={onNext}
                    disabled={!hasInstalled}
                >
                    Next
                </button>
            </div>

            <Show when={!hasInstalled && !detecting}>
                <div class="setup-wizard-no-cli">
                    No CLI tools detected. Install one to continue.
                </div>
            </Show>
        </div>
    );
};

DetectStep.displayName = "DetectStep";

// --- CLI Result Card ---

const CliResultCard = ({ result }: { result: CliDetectionResult }): JSX.Element => {
    const providerDef = PROVIDERS[result.provider];
    const [showInstall, setShowInstall] = createSignal(false);

    return (
        <div class={`setup-wizard-cli-card ${result.installed ? "installed" : "missing"}`}>
            <div class="setup-wizard-cli-info">
                <div class="setup-wizard-cli-name">
                    {providerDef?.displayName || result.provider}
                </div>
                <div class="setup-wizard-cli-status">
                    <Show
                        when={result.installed}
                        fallback={
                            <>
                                <span class="status-dot missing" />
                                <span class="status-text">Not installed</span>
                            </>
                        }
                    >
                        <span class="status-dot installed" />
                        <span class="status-text">
                            Installed{result.version ? ` (${result.version})` : ""}
                        </span>
                    </Show>
                </div>
                <Show when={result.path}>
                    <div class="setup-wizard-cli-path">{result.path}</div>
                </Show>
            </div>
            <Show when={!result.installed && providerDef}>
                <div class="setup-wizard-cli-install">
                    <button
                        class="setup-wizard-btn small"
                        onClick={() => setShowInstall(!showInstall())}
                    >
                        {showInstall() ? "Hide" : "Install"}
                    </button>
                    <Show when={showInstall()}>
                        <div class="setup-wizard-install-info">
                            <code>{isWindows() ? providerDef.windowsInstallCommand : providerDef.unixInstallCommand}</code>
                            <a
                                href="#"
                                onClick={(e) => {
                                    e.preventDefault();
                                    getApi().openExternal(providerDef.docsUrl);
                                }}
                            >
                                Docs
                            </a>
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    );
};

CliResultCard.displayName = "CliResultCard";

// --- Select Step ---

const SelectStep = ({
    results,
    selectedProvider,
    onSelect,
    onBack,
    onNext,
}: {
    results: CliDetectionResult[];
    selectedProvider: string;
    onSelect: (provider: string) => void;
    onBack: () => void;
    onNext: () => void;
}): JSX.Element => {
    const installedProviders = results.filter((r) => r.installed);

    return (
        <div class="setup-wizard-select">
            <div class="setup-wizard-section-title">Choose Your Provider</div>
            <div class="setup-wizard-section-desc">
                Select which AI assistant to use as your default.
            </div>

            <div class="setup-wizard-provider-list">
                <For each={installedProviders}>
                    {(result) => {
                        const providerDef = PROVIDERS[result.provider];
                        const isSelected = selectedProvider === result.provider;

                        return (
                            <label
                                class={`setup-wizard-provider-card ${isSelected ? "selected" : ""}`}
                            >
                                <input
                                    type="radio"
                                    name="provider"
                                    value={result.provider}
                                    checked={isSelected}
                                    onChange={() => onSelect(result.provider)}
                                />
                                <div class="setup-wizard-provider-info">
                                    <div class="setup-wizard-provider-name">
                                        {providerDef?.displayName || result.provider}
                                    </div>
                                    <div class="setup-wizard-provider-detail">
                                        {result.version || result.path || ""}
                                    </div>
                                </div>
                            </label>
                        );
                    }}
                </For>
            </div>

            <div class="setup-wizard-actions">
                <button class="setup-wizard-btn secondary" onClick={onBack}>
                    Back
                </button>
                <button
                    class="setup-wizard-btn primary"
                    onClick={onNext}
                    disabled={!selectedProvider}
                >
                    Next
                </button>
            </div>
        </div>
    );
};

SelectStep.displayName = "SelectStep";

// --- Confirm Step ---

const ConfirmStep = ({
    selectedProvider,
    detectionResult,
    onBack,
    onConfirm,
}: {
    selectedProvider: string;
    detectionResult?: CliDetectionResult;
    onBack: () => void;
    onConfirm: () => void;
}): JSX.Element => {
    const providerDef = PROVIDERS[selectedProvider];

    return (
        <div class="setup-wizard-confirm">
            <div class="setup-wizard-section-title">Ready to Go</div>
            <div class="setup-wizard-section-desc">Review your configuration and start.</div>

            <div class="setup-wizard-summary">
                <div class="setup-wizard-summary-row">
                    <span class="setup-wizard-summary-label">Provider</span>
                    <span class="setup-wizard-summary-value">
                        {providerDef?.displayName || selectedProvider}
                    </span>
                </div>
                <div class="setup-wizard-summary-row">
                    <span class="setup-wizard-summary-label">Command</span>
                    <span class="setup-wizard-summary-value">
                        <code>
                            {providerDef?.cliCommand}{" "}
                            {providerDef?.defaultArgs.join(" ")}
                        </code>
                    </span>
                </div>
                <Show when={detectionResult?.version}>
                    <div class="setup-wizard-summary-row">
                        <span class="setup-wizard-summary-label">Version</span>
                        <span class="setup-wizard-summary-value">
                            {detectionResult.version}
                        </span>
                    </div>
                </Show>
                <Show when={detectionResult?.path}>
                    <div class="setup-wizard-summary-row">
                        <span class="setup-wizard-summary-label">Path</span>
                        <span class="setup-wizard-summary-value">
                            {detectionResult.path}
                        </span>
                    </div>
                </Show>
                <div class="setup-wizard-summary-row">
                    <span class="setup-wizard-summary-label">Auth</span>
                    <span class="setup-wizard-summary-value">
                        {providerDef?.authType === "oauth" ? "OAuth (browser)" : "API Key"}
                    </span>
                </div>
            </div>

            <div class="setup-wizard-actions">
                <button class="setup-wizard-btn secondary" onClick={onBack}>
                    Back
                </button>
                <button class="setup-wizard-btn primary start" onClick={onConfirm}>
                    Start
                </button>
            </div>
        </div>
    );
};

ConfirmStep.displayName = "ConfirmStep";
