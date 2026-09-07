// Test-only CDP boundary. The adapter, React UI, redirect, and account linker remain real.
export const fakeSdk = `
import React, { useSyncExternalStore } from 'react';
const listeners = new Set();
const signedIn = () => sessionStorage.getItem('test-cdp-signed-in') === 'true';
const user = () => signedIn() ? { evmAccountObjects: [{address:'0x'+'22'.repeat(20)}], authenticationMethods: {oauth:{google:true}} } : null;
const subscribe = callback => { listeners.add(callback); return () => listeners.delete(callback); };
const login = redirect => {
  sessionStorage.setItem('test-cdp-signed-in','true');
  if (redirect) location.assign('/?test-oauth-return=1');
  else listeners.forEach(callback => callback());
};
export const CDPReactProvider = ({children}) => children;
export const useIsInitialized = () => ({isInitialized:true});
export const useIsSignedIn = () => ({isSignedIn:useSyncExternalStore(subscribe,signedIn)});
const testUser = { evmAccountObjects: [{address:'0x'+'22'.repeat(20)}], authenticationMethods: {oauth:{google:true}} };
export const useCurrentUser = () => ({currentUser:useSyncExternalStore(subscribe,()=>signedIn()?testUser:null)});
export const getCurrentUser = async () => user();
export const isSignedIn = async () => signedIn();
export const signOut = async () => { sessionStorage.removeItem('test-cdp-signed-in'); listeners.forEach(callback=>callback()); };
export const createCDPEmbeddedWallet = () => ({provider:{request:async request=>{
  window.walletTestCalls.push({wallet:'embedded',...request});
  return ['eth_accounts','eth_requestAccounts'].includes(request.method) ? (signedIn() ? ['0x'+'22'.repeat(20)] : []) : '0x'+'cd'.repeat(65);
}}});
export const SignIn = () => React.createElement('div',null,
  React.createElement('button',{onClick:()=>login(true)},'Continue with Google'),
  React.createElement('button',{onClick:()=>login(false)},'Complete email verification')
);
export const SignInBackButton = () => null;
export const SignInDescription = () => null;
export const SignInForm = () => null;
export const SignInAuthMethodButtons = () => null;
export const SignInFooter = () => null;
export const LinkAuth = () => null;
export const LinkAuthError = () => null;
export const LinkAuthFlow = () => null;
export const LinkAuthFlowBackButton = () => null;
export const LinkAuthTitle = () => null;
`;
