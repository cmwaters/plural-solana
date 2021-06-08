use std::{
    collections::HashMap,
    result::Result as ResultGeneric,
};

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_spl::token::{self, TokenAccount, Transfer};

#[program]
pub mod plural {
    use super::*;
    pub fn initialize(
        ctx: Context<InitializeMsg>,
        roles: Vec<Role>,
        members: Vec<Member>,
        policy: Policy,
        authority: Option<Pubkey>,
    ) -> ProgramResult {
        let mut member_id: u32 = 0;
        let mut total_weight: u32 = 0;
        let mut memberset: HashMap<u32, Member> = HashMap::new(); 
        let mut keyset: HashMap<Pubkey, u32> = HashMap::new();
        for m in members.into_iter() {
            memberset.insert(member_id, m.clone());
            keyset.insert(m.account.clone(), member_id);
            member_id += 1;
            total_weight += m.weight;
        }

        let polity = &mut ctx.accounts.polity;
        polity.members = memberset;
        polity.keys = keyset;
        polity.roles = roles;
        polity.authority = authority;
        polity.vault = *ctx.accounts.vault.to_account_info().key;
        polity.policy = policy;
        polity.total_weight = total_weight;
        Ok(())
    }

    pub fn propose(
        ctx: Context<ProposeMsg>,
        deposit_amount: u64,
        actions: Vec<Action>, 
        forum: String,
        content: String,
    ) -> ProgramResult {
        // assert that the deposit amount is correct
        if deposit_amount != ctx.accounts.polity.policy.deposit {
            return Err(ErrorCode::IncorrectDeposit)?;
        }

        // check that the proposer has permission
        if !ctx.accounts.polity.policy.public {
            let proposer_id = ctx.accounts.polity.keys.get(ctx.accounts.proposer.to_account_info().key);
            if proposer_id.is_none() {
                return Err(ErrorCode::MemberNotFound)?;
            }

            if !ctx.accounts.polity.has_permission(proposer_id.unwrap(), &actions) {
                return Err(ErrorCode::Unauthorized)?;
            }
        }

        // transfer the deposit across to the vault
        let cpi_accounts = Transfer {
            from: ctx.accounts.depositor.to_account_info().clone(),
            to: ctx.accounts.vault.to_account_info().clone(),
            authority: ctx.accounts.proposer.clone(),
        };
        let cpi_program = ctx.accounts.polity.to_account_info().clone();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::transfer(cpi_ctx, deposit_amount)?;
        ctx.accounts.polity.total_deposit_amount += deposit_amount;

        // create the proposal
        let proposal = &mut ctx.accounts.proposal;
        proposal.forum = forum;
        proposal.versions = vec![ProposalVersion {
            proposer: *ctx.accounts.proposer.to_account_info().key,
            actions: actions,
            content: content,
        }];
        proposal.tally = Tally::new();
        proposal.votes = HashMap::new();
        proposal.vetoed = Vec::new();
        proposal.submission_time = ctx.accounts.clock.unix_timestamp;
        proposal.status = ProposalStatus::InProgress;

        Ok(())
    }

    pub fn vote(
        ctx: Context<VoteMsg>,
        choice: Choice,
        tokens: u64,
    ) -> ProgramResult {
        let polity = &mut ctx.accounts.polity;
        let proposal = &mut ctx.accounts.proposal;
        if polity.check_expired(&proposal, ctx.accounts.clock.unix_timestamp) {
            proposal.status = ProposalStatus::Expired; 
            return Err(ErrorCode::ProposalExpired)?;
        }

        if proposal.status != ProposalStatus::InProgress {
            return Err(ErrorCode::ProposalFinished)?;
        }

        // TODO: check permissions

        let member_id: u32 = polity.keys.get(ctx.accounts.voter.to_account_info().key).unwrap().clone();
        let mut member = polity.members.get_mut(&member_id).unwrap();
    
        if tokens > member.tokens {
            return Err(ErrorCode::InsufficientTokens)?;
        }
        member.tokens -= tokens;

        let vote = Vote {
            choice,
            tokens
        };
        match proposal.votes.insert(member_id.clone(), vote.clone()) {
            None => {},
            Some(old_vote) => proposal.tally.remove_vote(&old_vote)
        }
        proposal.tally.add_vote(&vote);
        proposal.status = proposal.count_votes(polity.get_threshold());
        match proposal.status {
            ProposalStatus::Defeated => {
                proposal.execute_defeated();
            },
            ProposalStatus::Passed{ version} => {
                proposal.execute_passed(version);
            },
            _ => {},
        }


        Ok(())
    }

    pub fn execute(
        ctx: Context<ExecuteMsg>,
    ) -> ProgramResult {
        if ctx.accounts.proposal.status != ProposalStatus::InProgress {
            return Err(ErrorCode::ProposalFinished)?;
        }

        Ok(())
    }
}


// *********************************** MESSAGES **********************************

#[derive(Accounts)]
pub struct InitializeMsg<'info> {
    #[account(init)]
    polity: ProgramAccount<'info, Polity>,
    #[account(mut)]
    vault: CpiAccount<'info, TokenAccount>,
    rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct ProposeMsg<'info> {
    #[account(mut, has_one = vault)]
    polity: ProgramAccount<'info, Polity>,
    #[account(init)]
    proposal: ProgramAccount<'info, Proposal>,
    #[account(signer)]
    proposer: AccountInfo<'info>,
    #[account(mut)]
    vault: CpiAccount<'info, TokenAccount>,
    #[account(mut)]
    depositor: AccountInfo<'info>,
    rent: Sysvar<'info, Rent>,
    clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
pub struct VoteMsg<'info> {
	#[account(mut)]
	polity: ProgramAccount<'info, Polity>,
	#[account(signer)]
	voter: AccountInfo<'info>,
	#[account(mut, has_one = polity)]
	proposal: ProgramAccount<'info, Proposal>,
    clock: Sysvar<'info, Clock>,
}

#[derive(Accounts)]
pub struct ExecuteMsg<'info> {
    #[account(mut)]
    polity: ProgramAccount<'info, Polity>,
    #[account(signer)]
    caller: AccountInfo<'info>,
    #[account(mut, has_one = polity)]
	proposal: ProgramAccount<'info, Proposal>,
}

// *********************************** STATE **********************************

#[account]
pub struct Polity {
	// all the members within the polity
	members: HashMap<u32, Member>,
    keys: HashMap<Pubkey, u32>,

	// the collective account shared amongst the members
	vault: Pubkey,

    // an optional authority that can freely change policy and membership
    authority: Option<Pubkey>, 

    /// the policy of the group
    policy: Policy,
	
	// array of possible roles within the group. NOTE: roles can not be removed
	// from a polity completely, just from individual members.
	roles: Vec<Role>,

    // the total weight of all members in the polity
    total_weight: u32,

	// used to store the amount being deposited in active proposals. 
    // We keep this separate so that they are not spent
	total_deposit_amount: u64,
}

impl Polity {
    // TODO: implement permissions
    fn has_permission(&self, _member_id: &u32, _actions: &Vec<Action>) -> bool {
        true
    }

    fn get_threshold(&self) -> u64 {
        match &self.policy.threshold {
            NumberOrRatio::Number{number} => return number.clone(),
            NumberOrRatio::Ratio{ratio} => {
                let r = ratio.clone();
                return r.numerator * self.total_weight as u64 / r.denominator
            }
        }
    }

    fn check_expired(&self, proposal: &Proposal, now: i64) -> bool {
        now > proposal.submission_time + self.policy.election_period
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct Member {
	// the account pubkey (this could theoretically be another DAO)
	account: Pubkey,

	// the accumulated voting power
	tokens: u64,

	// the weight of the member (this is rate at which tokens accumulate) 
	weight: u32,

	// array of indexes corresponding to roles that the member has
	role_codes: Vec<u8>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct Role {
    // a list of permissions that the role has
	permissions: Vec<Permission>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct Permission {
	// enum: external | policy | membership
	proposal_type: ProposalType,

	// optional pub key for external proposal types
	program_id: Option<Pubkey>
}

#[account]
pub struct Proposal {
	/// unique id of the polity associated with the proposal
	polity: Pubkey,

    /// array of versions. referring to specific amendments when voting is 
    /// done based of the index.  
    versions: Vec<ProposalVersion>,
	
	/// a static pointer to the forum, discussing the proposal
	forum: String,

	/// voteset tracks who has voted, what they have voted on and the amount
	/// of tokens they have committed to that proposal
	votes: HashMap<u32, Vote>,

    /// members who have vetoed this proposal
    vetoed: Vec<u32>,
	
	/// keeps track of the running totals for each version
	tally: Tally,

    /// the time that the proposal was submitted
    submission_time: i64,

    /// the current status of the proposal
    status: ProposalStatus
}

impl Proposal {
    fn count_votes(&self, threshold: u64) -> ProposalStatus {
        if self.tally.reject >= threshold {
            return ProposalStatus::Defeated
        }

        for (index, approvals) in self.tally.approve.iter().enumerate() {
            if approvals > &threshold {
                return ProposalStatus::Passed{version: index as u8}
            }
        }

        ProposalStatus::InProgress
    }

    fn execute_passed(&self, version: u8) -> ProgramResult {
        Ok(())
    }

    fn execute_defeated(&self) -> ProgramResult {
        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct ProposalVersion {
    // pubkey or index of the member who proposed the proposal
    proposer: Pubkey,

    // the set of actions to to be executed by the program account on passing
    actions: Vec<Action>,

    // a static pointer, most likely IPFS or a URL, to further content or 
	// justification regarding the genesis proposal
    content: String,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct Policy {
    threshold: NumberOrRatio,
    election_period: i64,
    deposit: u64,
    public: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub enum NumberOrRatio {
    Number{number: u64},
    Ratio{ratio: Ratio},
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct Ratio {
    numerator: u64,
    denominator: u64
}

// Action is a mirror of Instruction
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct Action {
    /// The target program to execute against
    program_id: Pubkey,
    /// The accounts required for the instructions
    accounts: Vec<ActionAccount>, 
    /// The instruction data
    data: Vec<u8>,
}

impl From<&Action> for Instruction {
    fn from(action: &Action) -> Instruction {
        Instruction {
            program_id: action.program_id,
            accounts: action.accounts.clone().into_iter().map(Into::into).collect(),
            data: action.data.clone(),
        }
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct ActionAccount {
    pubkey: Pubkey,
    is_signer: bool,
    is_writable: bool,
}

impl From<ActionAccount> for AccountMeta {
    fn from(account: ActionAccount) -> AccountMeta {
        match account.is_writable {
            false => AccountMeta::new_readonly(account.pubkey, account.is_signer),
            true => AccountMeta::new(account.pubkey, account.is_signer),
        }
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum ProposalType {
    External = 0x01,
    Membership = 0x02,
    Policy = 0x03,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct Vote {
    choice: Choice,
    tokens: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub enum Choice {
    Approve{version: u8},
    Reject{},
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct Tally {
    // indexed by version id. keeps track of the running total for each version
    approve: Vec<u64>,
    reject: u64,
    veto: u64,
}

impl Tally {
    fn new() -> Tally {
        Tally {
            approve: Vec::new(),
            reject: 0,
            veto: 0,
        }
    }

    fn remove_vote(&mut self, vote: &Vote) {
        match vote.choice {
            Choice::Approve{version} => self.approve[version as usize] -= vote.tokens,
            Choice::Reject{} => self.reject -= vote.tokens
        }
    }

    fn add_vote(&mut self, vote: &Vote) {
        match vote.choice {
            Choice::Approve{version} => self.approve[version as usize] += vote.tokens,
            Choice::Reject{} => self.reject += vote.tokens
        }
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum ProposalStatus {
    InProgress,
    Passed{ version: u8 },
    Defeated,
    Expired,
    Removed,
    Withdrawn,
}

pub type ProposalResult = ResultGeneric<ProposalStatus, ProgramError>;

#[error]
pub enum ErrorCode {
    #[msg("Incorrect deposit amount specified")]
    IncorrectDeposit,
    #[msg("Proposer ID is required for a non public proposal")]
    RequireProposerID,
    #[msg("Member not found")]
    MemberNotFound,
    #[msg("Member is not authorized to perform that action")]
    Unauthorized,
    #[msg("Member has insufficient tokens to perform action")]
    InsufficientTokens, 
    #[msg("Proposal is no longer in progress")]
    ProposalFinished,
    #[msg("Proposal has expired")]
    ProposalExpired,
}
