import type { PageLoad } from './$types';
import { validateApiKey } from '$lib/utils';

export const ssr = false;
export const prerender = false;

export const load: PageLoad = async ({ fetch }) => {
	await validateApiKey(fetch);
};
