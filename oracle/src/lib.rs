#![cfg_attr(not(feature = "std"), no_std)]
#![feature(vec_remove_item)]

use support::{decl_event, decl_module, decl_storage, dispatch::Result, StorageMap, StorageValue};
use system::{ensure_signed, ensure_root};
use support::traits::{Get, ChangeMembers, Currency, LockableCurrency, ReservableCurrency, LockIdentifier, WithdrawReasons};
use sr_primitives::traits::{EnsureOrigin, CheckedSub, CheckedAdd, Zero, Bounded};
use rstd::prelude::*;
use codec::{Encode, Decode};

#[cfg(test)]
mod oracle_test;

type BalanceOf<T> = <<T as Trait>::Currency as Currency<<T as system::Trait>::AccountId>>::Balance;
const LockedId: LockIdentifier = *b"oracle  ";

pub trait Trait: system::Trait {
    type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;

    type Currency:
        LockableCurrency<Self::AccountId, Moment=Self::BlockNumber>
        + ReservableCurrency<Self::AccountId>;


    type OracleFee: Get<BalanceOf<Self>>;
    type MissReportSlash: Get<BalanceOf<Self>>;
    type MaliciousSlash: Get<BalanceOf<Self>>;
    type MinStaking: Get<BalanceOf<Self>>;

    type MaliciousSlashOrigin: EnsureOrigin<Self::Origin>;

    type Count: Get<u16>;

    type ReportInteval: Get<Self::BlockNumber>;
    type EraDuration: Get<Self::BlockNumber>;
    type LockedDuration: Get<Self::BlockNumber>;

    type ChangeMembers: ChangeMembers<Self::AccountId>;
}

pub trait OracleMixedIn<T: system::Trait> {
    fn on_witnessed(who: &T::AccountId);
    fn is_valid(who: &T::AccountId) -> bool;
}

#[derive(PartialEq, Eq, Clone, Encode, Decode)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct Unbind<Balance, BlockNumber> {
    amount: Balance,
    era: BlockNumber,
}

#[derive(PartialEq, Eq, Clone, Encode, Decode)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct Ledger<Balance: Default, BlockNumber> {
    active: Balance,
    unbonds: Vec<Unbind<Balance, BlockNumber>>,
}

impl<Balance: Default, BlockNumber> Default for Ledger<Balance, BlockNumber>{
    fn default() -> Self{
        Ledger{
            active: Balance::default(),
            unbonds: vec![],
        }
    }
}

decl_storage! {
    trait Store for Module<T: Trait> as OracleStorage {
        Oracles get(oracles): Vec<T::AccountId>;
        OracleLedger get(oracle_ledger): map T::AccountId => Ledger<BalanceOf<T>, T::BlockNumber>;
        WitnessReport get(witness_report): map T::AccountId => T::BlockNumber;
        OracleCandidates get(candidates): Vec<T::AccountId>;
        CurrentEra get(current_era): T::BlockNumber;
    }
}

decl_module! {
    pub struct Module<T: Trait> for enum Call where origin: T::Origin {
        fn deposit_event() = default;

        const OracleFee: BalanceOf<T> = T::OracleFee::get();
        const MissReportSlash: BalanceOf<T> = T::MissReportSlash::get();
        const MaliciousSlash: BalanceOf<T> = T::MaliciousSlash::get();
        const MinStaking: BalanceOf<T> = T::MinStaking::get();
        const Count: u16 = T::Count::get();
        const EraDuration: T::BlockNumber = T::EraDuration::get();
        const ReportInteval: T::BlockNumber = T::ReportInteval::get();
        const LockedDuration: T::BlockNumber = T::LockedDuration::get();


        pub fn bid(origin, amount: BalanceOf<T>) -> Result{
            let who = ensure_signed(origin)?;
            Self::bind(&who, amount)?;
            Self::add_candidates(&who)?;
            Ok(())
        }

        pub fn slash_by_vote(origin, who: T::AccountId, amount: BalanceOf<T>) -> Result{
            T::MaliciousSlashOrigin::try_origin(origin)
                .map(|_| ())
                .or_else(ensure_root)
                .map_err(|_| "bad origin")?;
            T::Currency::slash(&who, amount);
            Self::deposit_event(RawEvent::OracleSlashed(who, amount));
            Ok(())
        }

        pub fn unbond(origin, amount: BalanceOf<T>) -> Result{
            let who = ensure_signed(origin)?;
            Self::unbind(&who, amount)
        }
    }
}

impl<T: Trait> Module<T>{
    pub fn on_finalize(block_number: T::BlockNumber) {
        Self::slash_oracles(block_number);

        let current_era = Self::current_era();
        if block_number >= current_era + T::EraDuration::get(){
            Self::elect_oracles();
            <CurrentEra<T>>::put(current_era+T::EraDuration::get());
        }
    }

    fn slash_oracles(block_number: T::BlockNumber){
        let current_oracles = Self::oracles();

        current_oracles.iter().for_each(|o| {
            let last_report_height = Self::witness_report(o);
            if block_number > last_report_height + T::ReportInteval::get(){
                Self::slash(o, T::MissReportSlash::get());
            }
        });
    }

    fn elect_oracles(){
        let current_oracles = Self::oracles();
        let new_candidates = Self::candidates();
        let mut all_candidates: Vec<T::AccountId> = Vec::new();

        all_candidates.extend(new_candidates);
        all_candidates.extend(current_oracles.clone());

        let all_candidates: Vec<(&T::AccountId, Ledger<BalanceOf<T>, T::BlockNumber>)> = all_candidates.iter().map(|a| {
            let ledger = Self::oracle_ledger(a);
            (a, ledger)
        }).collect();

        all_candidates.iter().for_each(|(a, ledger)|{
        });

        let mut all_candidates: Vec<(&T::AccountId, Ledger<BalanceOf<T>, T::BlockNumber>)> = all_candidates.into_iter().
            filter(|(_, ledger)| ledger.active > Zero::zero()).
            collect();
        all_candidates.sort_by_key(|(_, ledger)| ledger.active);

        let all_candidates = all_candidates.into_iter().  map(|(a, _)| a.clone()).  collect::<Vec<T::AccountId>>();
        let (chosen_candidates, new_candidates) = all_candidates.split_at(T::Count::get().into());

        let mut chosen_candidates = chosen_candidates.to_vec();
        chosen_candidates.sort();

        let new_oracles: Vec<T::AccountId> = chosen_candidates.clone().into_iter().filter(|o| !current_oracles.contains(&o)).collect();
        let outgoing_oracles: Vec<T::AccountId> = current_oracles.into_iter().filter(|o| !new_oracles.contains(&o)).collect();
        <Oracles<T>>::put(&chosen_candidates); 
        T::ChangeMembers::change_members(&new_oracles, &outgoing_oracles, chosen_candidates);
        <OracleCandidates<T>>::put(new_candidates.to_vec());
    }
}

impl<T:Trait> Module<T>{
    fn oracle_stakes(who: &T::AccountId) -> BalanceOf<T>{
        let ledger = Self::oracle_ledger(who);
        ledger.active
    }

    fn slash(who: &T::AccountId, amount: BalanceOf<T>) -> Result {
        let free_balance = T::Currency::free_balance(who);

        if free_balance < amount {
            // Remove this oracle
            let mut current_oracles = Self::oracles();
            current_oracles.remove_item(&who);
            <Oracles <T>>::put(&current_oracles);
            T::ChangeMembers::change_members(&[], &[who.clone()], current_oracles);
        }

        // Handle imbalance
        T::Currency::slash(who, amount);

        Self::deposit_event(RawEvent::OracleSlashed(who.clone(), amount));
        Ok(())
    }

    fn unbind(who: &T::AccountId, amount: BalanceOf<T>) -> Result{
        let current_height = <system::Module<T>>::block_number();
        let mut ledger = Self::oracle_ledger(who);

        if amount > ledger.active {
            return Err("staking amount is smaller than unbonding amount");
        }

        let new_unbond = Unbind{
            amount: amount,
            era: current_height + T::LockedDuration::get(),
        };

        ledger.active = ledger.active.checked_sub(&amount).ok_or("Error calculating new staking")?;
        ledger.unbonds.push(new_unbond);

        <OracleLedger<T>>::insert(who, ledger);
        Self::deposit_event(RawEvent::OracleUnbonded(who.clone(), amount));
        Ok(())
    }

    fn bind(who: &T::AccountId, amount: BalanceOf<T>) -> Result{
        let mut ledger = Self::oracle_ledger(who);
        let new_staked = ledger.active.checked_add(&amount).ok_or("Error calculating new staking")?;
        ledger.active = new_staked;
        <OracleLedger<T>>::insert(who, ledger);
        T::Currency::set_lock(
            LockedId,
            &who,
            amount,
            T::BlockNumber::max_value(),
            WithdrawReasons::all(),
        );
        Self::deposit_event(RawEvent::OracleBonded(who.clone(), amount));
        Ok(())
    }

    fn add_candidates(who: &T::AccountId) -> Result{
        let mut candidates = Self::candidates();
        if !candidates.contains(&who) {
            candidates.push(who.clone());
            <OracleCandidates<T>>::put(candidates);
            Self::deposit_event(RawEvent::CandidatesAdded(who.clone()));
        }
        Ok(())
    }
}

impl<T: Trait> OracleMixedIn<T> for Module<T> {
    fn on_witnessed(who: &T::AccountId){
        let current_height = <system::Module<T>>::block_number();
        <WitnessReport<T>>::insert(who, current_height);
    }

    fn is_valid(who: &T::AccountId) -> bool{
        let report_height = Self::witness_report(who);
        report_height + T::ReportInteval::get() >= <system::Module<T>>::block_number()
    }

}

decl_event!(
    pub enum Event<T>
    where
        AccountId = <T as system::Trait>::AccountId,
        Balance = BalanceOf<T>,
    {
        OracleBonded(AccountId, Balance),
        OracleUnbonded(AccountId, Balance),
        OracleSlashed(AccountId, Balance),

        CandidatesAdded(AccountId),
        CandidatesRemoved(AccountId),

    }
);