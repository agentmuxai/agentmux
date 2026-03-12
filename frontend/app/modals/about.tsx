// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import logoUrl from "@/app/asset/logo.svg?url";
import { modalsModel } from "@/app/store/modalmodel";
import { Modal } from "./modal";

import { isDev } from "@/util/isdev";
import { getApi } from "../store/global";

interface AboutModalProps {}

const AboutModal = ({}: AboutModalProps) => {
    const currentDate = new Date();
    const details = getApi().getAboutModalDetails();
    const updaterChannel = getApi().getUpdaterChannel();

    return (
        <Modal class="pb-[34px]" onClose={() => modalsModel.popModal()}>
            <div class="flex flex-col gap-[26px] w-full">
                <div class="flex flex-col items-center justify-center gap-4 self-stretch w-full text-center">
                    <img src={logoUrl} style={{ height: "48px" }} />
                    <div class="text-[25px]">AgentMux</div>
                    <div class="leading-5">
                        Open-Source AI-Native Terminal
                        <br />
                        Built for Seamless Workflows
                    </div>
                </div>
                <div class="items-center gap-4 self-stretch w-full text-center">
                    Client Version {details.version} ({isDev() ? "dev-" : ""}
                    {details.buildTime})
                    <br />
                    Update Channel: {updaterChannel}
                </div>
                <div class="flex items-start gap-[10px] self-stretch w-full text-center">
                    <a
                        href="https://github.com/agentmuxai/agentmux?ref=about"
                        target="_blank"
                        rel="noopener"
                        class="inline-flex items-center px-4 py-2 rounded border border-border hover:bg-hoverbg transition-colors duration-200"
                    >
                        <i class="fa-brands fa-github mr-2"></i>Github
                    </a>
                    <a
                        href="https://github.com/agentmuxai/agentmux?ref=about"
                        target="_blank"
                        rel="noopener"
                        class="inline-flex items-center px-4 py-2 rounded border border-border hover:bg-hoverbg transition-colors duration-200"
                    >
                        <i class="fa-sharp fa-light fa-globe mr-2"></i>Website
                    </a>
                    <a
                        href="https://github.com/agentmuxai/agentmux/blob/main/ACKNOWLEDGEMENTS.md"
                        target="_blank"
                        rel="noopener"
                        class="inline-flex items-center px-4 py-2 rounded border border-border hover:bg-hoverbg transition-colors duration-200"
                    >
                        <i class="fa-sharp fa-light fa-heart mr-2"></i>Acknowledgements
                    </a>
                </div>
                <div class="items-center gap-4 self-stretch w-full text-center">
                    &copy; {currentDate.getFullYear()} Command Line Inc.
                </div>
            </div>
        </Modal>
    );
};

AboutModal.displayName = "AboutModal";

export { AboutModal };
