export interface OpportunityV2 {
  id: string;
  contractAddress: string;
  chainId: number;
  title: string;
  description: string;
  reward: {
    amount: string;
    token: string;
    decimals: number;
  };
  window: {
    opensAt: number;
    closesAt: number;
    timezone: string;
  };
  scoring: {
    objectiveInputs: string[];
    weightings: Record<string, number>;
    maxScore: number;
  };
  economics: {
    entryCost: string;
    gasEstimate: string;
    slippageTolerance: number;
  };
  risk: {
    level: 'low' | 'medium' | 'high';
    factors: string[];
  };
  proofSequence: {
    steps: ProofStep[];
    evidenceBoundary: string;
  };
  status: 'upcoming' | 'active' | 'closed' | 'finalized';
  canonicalLink: string;
  createdAt: number;
  updatedAt: number;
}

export interface ProofStep {
  id: string;
  name: string;
  description: string;
  requiredEvidence: string[];
  verificationMethod: 'automated' | 'manual' | 'hybrid';
  order: number;
}

export interface ParticipationWorkspace {
  opportunityId: string;
  contractAddress: string;
  chainId: number;
  timing: {
    windowOpensAt: number;
    windowClosesAt: number;
    currentPhase: 'pre-window' | 'active' | 'post-window' | 'finalizing';
    phaseEndsAt: number;
  };
  scoring: {
    objectiveInputs: ScoringInput[];
    weightings: Record<string, number>;
    maxScore: number;
    currentBestScore: number | null;
    scoreHistory: ScoreEntry[];
  };
  economics: {
    rewardAmount: string;
    rewardToken: string;
    rewardDecimals: number;
    entryCost: string;
    gasEstimate: string;
    slippageTolerance: number;
    estimatedNetReward: string;
  };
  risk: {
    level: 'low' | 'medium' | 'high';
    factors: RiskFactor[];
  };
  proofSequence: {
    steps: ProofStep[];
    evidenceBoundary: string;
    submittedEvidence: SubmittedEvidence[];
  };
  childBountyInstructions: ChildBountyInstruction[];
  canonicalEvidenceBoundary: string;
  participationState: 'not-started' | 'in-progress' | 'submitted' | 'verified' | 'rewarded' | 'expired';
}

export interface ScoringInput {
  key: string;
  label: string;
  type: 'numeric' | 'boolean' | 'categorical';
  description: string;
  weight: number;
  minValue?: number;
  maxValue?: number;
  categories?: string[];
}

export interface ScoreEntry {
  score: number;
  timestamp: number;
  evidenceHash: string;
}

export interface RiskFactor {
  id: string;
  label: string;
  severity: 'low' | 'medium' | 'high';
  description: string;
  mitigation?: string;
}

export interface SubmittedEvidence {
  stepId: string;
  evidenceHash: string;
  submittedAt: number;
  status: 'pending' | 'verified' | 'rejected';
}

export interface ChildBountyInstruction {
  id: string;
  title: string;
  description: string;
  rewardAmount: string;
  rewardToken: string;
  rewardDecimals: number;
  prefilledData: Record<string, unknown>;
  canonicalLink: string;
}

export interface MarketplaceReadiness {
  isReady: boolean;
  reason?: string;
  missingRequirements?: string[];
  contractChecks: ContractCheck[];
}

export interface ContractCheck {
  name: string;
  passed: boolean;
  details?: string;
}

export interface FunnelEvent {
  eventType: 'view' | 'start' | 'evidence_submit' | 'proof_step_complete' | 'abandon' | 'complete';
  opportunityId: string;
  contractAddress: string;
  chainId: number;
  timestamp: number;
  sessionId: string;
  metadata?: Record<string, unknown>;
}

export interface AbandonmentEvent {
  eventType: 'abandon';
  opportunityId: string;
  contractAddress: string;
  chainId: number;
  timestamp: number;
  sessionId: string;
  lastCompletedStep: string | null;
  timeSpent: number;
  reason?: 'timeout' | 'complexity' | 'cost' | 'technical' | 'other';
}

export interface PresentationVariant {
  type: 'reward' | 'window' | 'child_bounty_instructions';
  data: Record<string, unknown>;
  reviewable: boolean;
}
