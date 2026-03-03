// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import React, { memo, useState, useCallback, useEffect } from "react";
import { getApi } from "@/app/store/global";
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

export const SetupWizard: React.FC<SetupWizardProps> = memo(({ onSetupComplete }) => {
    const [step, setStep] = useState<WizardStep>("detect");
    const [detectionResults, setDetectionResults] = useState<CliDetectionResult[]>([]);
    const [detecting, setDetecting] = useState(false);
    const [selectedProvider, setSelectedProvider] = useState<string>("");
    const [error, setError] = useState<string | null>(null);

    const runDetection = useCallback(async () => {
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
    }, []);

    // Run detection on mount
    useEffect(() => {
        void runDetection();
    }, [runDetection]);

    const handleConfirm = useCallback(async () => {
        if (!selectedProvider) return;

        const providerDef = PROVIDERS[selectedProvider];
        if (!providerDef) return;

        const config: ProviderConfig = {
            default_provider: selectedProvider,
            providers: {
                [selectedProvider]: {
                    cli_path: detectionResults.find((r) => r.provider === selectedProvider)?.path ?? null,
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
    }, [selectedProvider, detectionResults, onSetupComplete]);

    const installedCount = detectionResults.filter((r) => r.installed).length;
    const hasInstalled = installedCount > 0;

    return (
        <div className="setup-wizard">
            <div className="setup-wizard-header">
                <div className="setup-wizard-title">Agent Setup</div>
                <div className="setup-wizard-subtitle">Configure your AI coding assistant</div>
                <div className="setup-wizard-steps">
                    <StepIndicator step="detect" label="Detect" currentStep={step} />
                    <div className="setup-wizard-step-connector" />
                    <StepIndicator step="select" label="Select" currentStep={step} />
                    <div className="setup-wizard-step-connector" />
                    <StepIndicator step="confirm" label="Start" currentStep={step} />
                </div>
            </div>

            <div className="setup-wizard-content">
                {error && (
                    <div className="setup-wizard-error">
                        <span className="setup-wizard-error-icon">!</span>
                        {error}
                    </div>
                )}

                {step === "detect" && (
                    <DetectStep
                        results={detectionResults}
                        detecting={detecting}
                        onRefresh={runDetection}
                        onNext={() => setStep("select")}
                        hasInstalled={hasInstalled}
                    />
                )}

                {step === "select" && (
                    <SelectStep
                        results={detectionResults}
                        selectedProvider={selectedProvider}
                        onSelect={setSelectedProvider}
                        onBack={() => setStep("detect")}
                        onNext={() => setStep("confirm")}
                    />
                )}

                {step === "confirm" && (
                    <ConfirmStep
                        selectedProvider={selectedProvider}
                        detectionResult={detectionResults.find((r) => r.provider === selectedProvider)}
                        onBack={() => setStep("select")}
                        onConfirm={handleConfirm}
                    />
                )}
            </div>
        </div>
    );
});

SetupWizard.displayName = "SetupWizard";

// --- Step Indicator ---

const StepIndicator: React.FC<{
    step: WizardStep;
    label: string;
    currentStep: WizardStep;
}> = memo(({ step, label, currentStep }) => {
    const steps: WizardStep[] = ["detect", "select", "confirm"];
    const current = steps.indexOf(currentStep);
    const index = steps.indexOf(step);
    const isActive = index === current;
    const isDone = index < current;

    return (
        <div
            className={`setup-wizard-step-indicator ${isActive ? "active" : ""} ${isDone ? "done" : ""}`}
        >
            <div className="setup-wizard-step-dot">
                {isDone ? "\u2713" : index + 1}
            </div>
            <div className="setup-wizard-step-label">{label}</div>
        </div>
    );
});

StepIndicator.displayName = "StepIndicator";

// --- Detect Step ---

const DetectStep: React.FC<{
    results: CliDetectionResult[];
    detecting: boolean;
    onRefresh: () => void;
    onNext: () => void;
    hasInstalled: boolean;
}> = memo(({ results, detecting, onRefresh, onNext, hasInstalled }) => {
    return (
        <div className="setup-wizard-detect">
            <div className="setup-wizard-section-title">Detected CLI Tools</div>
            <div className="setup-wizard-section-desc">
                Scanning for installed AI coding assistants...
            </div>

            <div className="setup-wizard-cli-list">
                {results.map((result) => (
                    <CliResultCard key={result.provider} result={result} />
                ))}
                {detecting && results.length === 0 && (
                    <div className="setup-wizard-detecting">Scanning...</div>
                )}
            </div>

            <div className="setup-wizard-actions">
                <button
                    className="setup-wizard-btn secondary"
                    onClick={onRefresh}
                    disabled={detecting}
                >
                    {detecting ? "Scanning..." : "Refresh"}
                </button>
                <button
                    className="setup-wizard-btn primary"
                    onClick={onNext}
                    disabled={!hasInstalled}
                >
                    Next
                </button>
            </div>

            {!hasInstalled && !detecting && (
                <div className="setup-wizard-no-cli">
                    No CLI tools detected. Install one to continue.
                </div>
            )}
        </div>
    );
});

DetectStep.displayName = "DetectStep";

// --- CLI Result Card ---

const CliResultCard: React.FC<{ result: CliDetectionResult }> = memo(({ result }) => {
    const providerDef = PROVIDERS[result.provider];
    const [showInstall, setShowInstall] = useState(false);

    return (
        <div className={`setup-wizard-cli-card ${result.installed ? "installed" : "missing"}`}>
            <div className="setup-wizard-cli-info">
                <div className="setup-wizard-cli-name">
                    {providerDef?.displayName || result.provider}
                </div>
                <div className="setup-wizard-cli-status">
                    {result.installed ? (
                        <>
                            <span className="status-dot installed" />
                            <span className="status-text">
                                Installed{result.version ? ` (${result.version})` : ""}
                            </span>
                        </>
                    ) : (
                        <>
                            <span className="status-dot missing" />
                            <span className="status-text">Not installed</span>
                        </>
                    )}
                </div>
                {result.path && (
                    <div className="setup-wizard-cli-path">{result.path}</div>
                )}
            </div>
            {!result.installed && providerDef && (
                <div className="setup-wizard-cli-install">
                    <button
                        className="setup-wizard-btn small"
                        onClick={() => setShowInstall(!showInstall)}
                    >
                        {showInstall ? "Hide" : "Install"}
                    </button>
                    {showInstall && (
                        <div className="setup-wizard-install-info">
                            <code>{providerDef.installCommand}</code>
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
                    )}
                </div>
            )}
        </div>
    );
});

CliResultCard.displayName = "CliResultCard";

// --- Select Step ---

const SelectStep: React.FC<{
    results: CliDetectionResult[];
    selectedProvider: string;
    onSelect: (provider: string) => void;
    onBack: () => void;
    onNext: () => void;
}> = memo(({ results, selectedProvider, onSelect, onBack, onNext }) => {
    const installedProviders = results.filter((r) => r.installed);

    return (
        <div className="setup-wizard-select">
            <div className="setup-wizard-section-title">Choose Your Provider</div>
            <div className="setup-wizard-section-desc">
                Select which AI assistant to use as your default.
            </div>

            <div className="setup-wizard-provider-list">
                {installedProviders.map((result) => {
                    const providerDef = PROVIDERS[result.provider];
                    const isSelected = selectedProvider === result.provider;

                    return (
                        <label
                            key={result.provider}
                            className={`setup-wizard-provider-card ${isSelected ? "selected" : ""}`}
                        >
                            <input
                                type="radio"
                                name="provider"
                                value={result.provider}
                                checked={isSelected}
                                onChange={() => onSelect(result.provider)}
                            />
                            <div className="setup-wizard-provider-info">
                                <div className="setup-wizard-provider-name">
                                    {providerDef?.displayName || result.provider}
                                </div>
                                <div className="setup-wizard-provider-detail">
                                    {result.version || result.path || ""}
                                </div>
                            </div>
                        </label>
                    );
                })}
            </div>

            <div className="setup-wizard-actions">
                <button className="setup-wizard-btn secondary" onClick={onBack}>
                    Back
                </button>
                <button
                    className="setup-wizard-btn primary"
                    onClick={onNext}
                    disabled={!selectedProvider}
                >
                    Next
                </button>
            </div>
        </div>
    );
});

SelectStep.displayName = "SelectStep";

// --- Confirm Step ---

const ConfirmStep: React.FC<{
    selectedProvider: string;
    detectionResult?: CliDetectionResult;
    onBack: () => void;
    onConfirm: () => void;
}> = memo(({ selectedProvider, detectionResult, onBack, onConfirm }) => {
    const providerDef = PROVIDERS[selectedProvider];

    return (
        <div className="setup-wizard-confirm">
            <div className="setup-wizard-section-title">Ready to Go</div>
            <div className="setup-wizard-section-desc">Review your configuration and start.</div>

            <div className="setup-wizard-summary">
                <div className="setup-wizard-summary-row">
                    <span className="setup-wizard-summary-label">Provider</span>
                    <span className="setup-wizard-summary-value">
                        {providerDef?.displayName || selectedProvider}
                    </span>
                </div>
                <div className="setup-wizard-summary-row">
                    <span className="setup-wizard-summary-label">Command</span>
                    <span className="setup-wizard-summary-value">
                        <code>
                            {providerDef?.cliCommand}{" "}
                            {providerDef?.defaultArgs.join(" ")}
                        </code>
                    </span>
                </div>
                {detectionResult?.version && (
                    <div className="setup-wizard-summary-row">
                        <span className="setup-wizard-summary-label">Version</span>
                        <span className="setup-wizard-summary-value">
                            {detectionResult.version}
                        </span>
                    </div>
                )}
                {detectionResult?.path && (
                    <div className="setup-wizard-summary-row">
                        <span className="setup-wizard-summary-label">Path</span>
                        <span className="setup-wizard-summary-value">
                            {detectionResult.path}
                        </span>
                    </div>
                )}
                <div className="setup-wizard-summary-row">
                    <span className="setup-wizard-summary-label">Auth</span>
                    <span className="setup-wizard-summary-value">
                        {providerDef?.authType === "oauth" ? "OAuth (browser)" : "API Key"}
                    </span>
                </div>
            </div>

            <div className="setup-wizard-actions">
                <button className="setup-wizard-btn secondary" onClick={onBack}>
                    Back
                </button>
                <button className="setup-wizard-btn primary start" onClick={onConfirm}>
                    Start
                </button>
            </div>
        </div>
    );
});

ConfirmStep.displayName = "ConfirmStep";
