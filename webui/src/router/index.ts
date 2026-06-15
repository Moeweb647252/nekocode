import { createRouter, createWebHistory } from 'vue-router'

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    {
      path: '/',
      name: 'main',
      component: () => import('../modules/main/View.vue'),
    },
    {
      path: '/settings',
      name: 'settings',
      component: () => import('../modules/settings/View.vue'),
    },
  ],
})

export default router
