import { createRouter, createWebHistory } from "vue-router";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", redirect: "/home" },
    {
      path: "/home",
      name: "home",
      component: () => import("../views/Home.vue"),
    },
    {
      path: "/subscriptions",
      name: "subscriptions",
      component: () => import("../views/Subscriptions.vue"),
    },
    {
      path: "/nodes",
      name: "nodes",
      component: () => import("../views/Nodes.vue"),
    },
    {
      path: "/connections",
      name: "connections",
      component: () => import("../views/Connections.vue"),
    },
    {
      path: "/stats",
      name: "stats",
      component: () => import("../views/Stats.vue"),
    },
    {
      path: "/logs",
      name: "logs",
      component: () => import("../views/Logs.vue"),
    },
    {
      path: "/rules",
      name: "rules",
      component: () => import("../views/Rules.vue"),
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("../views/Settings.vue"),
    },
  ],
});
