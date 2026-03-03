#!/bin/bash

if type update-alternatives 2>/dev/null >&1; then
    # Remove previous link if it doesn't use update-alternatives
    if [ -L '/usr/bin/agentmux' -a -e '/usr/bin/agentmux' -a "`readlink '/usr/bin/agentmux'`" != '/etc/alternatives/agentmux' ]; then
        rm -f '/usr/bin/agentmux'
    fi
    update-alternatives --install '/usr/bin/agentmux' 'agentmux' '/opt/AgentMux/agentmux' 100 || ln -sf '/opt/AgentMux/agentmux' '/usr/bin/agentmux'
else
    ln -sf '/opt/AgentMux/agentmux' '/usr/bin/agentmux'
fi

chmod 4755 '/opt/AgentMux/chrome-sandbox' || true

if hash update-mime-database 2>/dev/null; then
    update-mime-database /usr/share/mime || true
fi

if hash update-desktop-database 2>/dev/null; then
    update-desktop-database /usr/share/applications || true
fi
