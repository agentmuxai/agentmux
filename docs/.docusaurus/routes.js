import React from 'react';
import ComponentCreator from '@docusaurus/ComponentCreator';

export default [
  {
    path: '/docsite/',
    component: ComponentCreator('/docsite/', 'ba9'),
    routes: [
      {
        path: '/docsite/',
        component: ComponentCreator('/docsite/', '2a3'),
        routes: [
          {
            path: '/docsite/',
            component: ComponentCreator('/docsite/', '1cd'),
            routes: [
              {
                path: '/docsite/ai-presets',
                component: ComponentCreator('/docsite/ai-presets', '8f7'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/config',
                component: ComponentCreator('/docsite/config', 'a2a'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/connections',
                component: ComponentCreator('/docsite/connections', 'f42'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/customization',
                component: ComponentCreator('/docsite/customization', '866'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/customwidgets',
                component: ComponentCreator('/docsite/customwidgets', '8db'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/faq',
                component: ComponentCreator('/docsite/faq', 'a19'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/gettingstarted',
                component: ComponentCreator('/docsite/gettingstarted', '8c4'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/keybindings',
                component: ComponentCreator('/docsite/keybindings', '36a'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/layout',
                component: ComponentCreator('/docsite/layout', '170'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/presets',
                component: ComponentCreator('/docsite/presets', 'e74'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/releasenotes',
                component: ComponentCreator('/docsite/releasenotes', '49b'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/tabs',
                component: ComponentCreator('/docsite/tabs', '7df'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/telemetry',
                component: ComponentCreator('/docsite/telemetry', 'b75'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/telemetry-old',
                component: ComponentCreator('/docsite/telemetry-old', '07d'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/waveai',
                component: ComponentCreator('/docsite/waveai', '4b3'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/widgets',
                component: ComponentCreator('/docsite/widgets', '5bf'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/workspaces',
                component: ComponentCreator('/docsite/workspaces', '929'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/wsh',
                component: ComponentCreator('/docsite/wsh', '493'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/wsh-reference',
                component: ComponentCreator('/docsite/wsh-reference', '6c0'),
                exact: true,
                sidebar: "defaultSidebar"
              },
              {
                path: '/docsite/',
                component: ComponentCreator('/docsite/', '6cc'),
                exact: true,
                sidebar: "defaultSidebar"
              }
            ]
          }
        ]
      }
    ]
  },
  {
    path: '*',
    component: ComponentCreator('*'),
  },
];
