// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

mod conversions;
mod declared;
mod overlay;
mod wider_primitive_spellings;

use fhir_codegen::closure::TypeClosure;
use fhir_codegen::lower::VersionModule;
use fhir_codegen::operations::OperationContract;
use fhir_codegen::package::Package;
use fhir_codegen::roots::RootSet;

fn contracts(package: &Package, module: &str) -> Vec<OperationContract> {
    let roots = RootSet::select(package).expect("root set selects");
    let closure = TypeClosure::compute(package, &roots).expect("closure computes");
    let model = VersionModule::lower(&closure, module, "pkg", "0").expect("model lowers");
    let mut contracts: Vec<OperationContract> = roots
        .operations
        .values()
        .flat_map(|operation| {
            operation.resource.iter().map(|resource| {
                OperationContract::lower(operation, resource, &model).expect("contract lowers")
            })
        })
        .collect();
    contracts.sort_by(|a, b| a.module.cmp(&b.module));
    contracts
}

fn find<'a>(
    contracts: &'a [OperationContract],
    resource: &str,
    code: &str,
) -> &'a OperationContract {
    contracts
        .iter()
        .find(|c| c.resource == resource && c.code == code)
        .expect("operation exists")
}
