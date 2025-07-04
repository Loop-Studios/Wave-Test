use std::collections::HashMap;
use inkwell::{AddressSpace, FloatPredicate};
use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::types::{AnyTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue};
use parser::ast::{ASTNode, Expression, Literal, Mutability, StatementNode, VariableNode, WaveType};
use crate::llvm_temporary::expression::generate_expression_ir;
use crate::llvm_temporary::llvm_codegen::{generate_address_ir, wave_format_to_c, wave_type_to_llvm_type, VariableInfo};

pub fn generate_statement_ir<'ctx>(
    context: &'ctx Context,
    builder: &'ctx inkwell::builder::Builder<'ctx>,
    module: &'ctx inkwell::module::Module<'ctx>,
    string_counter: &mut usize,
    stmt: &ASTNode,
    variables: &mut HashMap<String, VariableInfo<'ctx>>,
    loop_exit_stack: &mut Vec<BasicBlock<'ctx>>,
    loop_continue_stack: &mut Vec<BasicBlock<'ctx>>,
    current_function: FunctionValue<'ctx>,
) {
    match stmt {
        ASTNode::Variable(VariableNode {
                              name,
                              type_name,
                              initial_value,
                              mutability
                          }) => unsafe {
            let llvm_type = wave_type_to_llvm_type(&context, &type_name);
            let alloca = builder.build_alloca(llvm_type, &name).unwrap();

            if let (WaveType::Array(element_type, size), Some(Expression::ArrayLiteral(values))) = (&type_name, &initial_value) {
                if values.len() != *size as usize {
                    panic!(
                        "❌ Array length mismatch: expected {}, got {}",
                        size,
                        values.len()
                    );
                }

                let llvm_element_type = wave_type_to_llvm_type(context, element_type);

                for (i, value_expr) in values.iter().enumerate() {
                    let value = generate_expression_ir(context, builder, value_expr, variables, module, Some(llvm_element_type));

                    let gep = builder.build_in_bounds_gep(
                        alloca,
                        &[
                            context.i32_type().const_zero(),
                            context.i32_type().const_int(i as u64, false),
                        ],
                        &format!("array_idx_{}", i),
                    ).unwrap();

                    builder.build_store(gep, value).unwrap();
                }

                variables.insert(
                    name.clone(),
                    VariableInfo {
                        ptr: alloca,
                        mutability: mutability.clone(),
                    },
                );

                return;
            }

            variables.insert(
                name.clone(),
                VariableInfo {
                    ptr: alloca,
                    mutability: mutability.clone(),
                },
            );

            if let Some(init) = initial_value {
                match (init, llvm_type) {
                    (Expression::Literal(Literal::Number(value)), BasicTypeEnum::IntType(int_type)) => {
                        let init_value = int_type.const_int(*value as u64, false);
                        let _ = builder.build_store(alloca, init_value);
                    }
                    (Expression::Literal(Literal::Float(value)), BasicTypeEnum::FloatType(float_type)) => {
                        let init_value = float_type.const_float(*value);
                        builder.build_store(alloca, init_value).unwrap();
                    }
                    (Expression::Literal(Literal::Float(value)), _) => {
                        let float_value = context.f32_type().const_float(*value);

                        let casted_value = match llvm_type {
                            BasicTypeEnum::IntType(int_ty) => {
                                builder.build_float_to_signed_int(float_value, int_ty, "float_to_int").unwrap().as_basic_value_enum()
                            }
                            BasicTypeEnum::FloatType(_) => float_value.as_basic_value_enum(),
                            _ => panic!("Unsupported type for float literal initialization"),
                        };

                        builder.build_store(alloca, casted_value).unwrap();
                    }
                    (Expression::Literal(Literal::String(value)), BasicTypeEnum::PointerType(_)) => unsafe {
                        let string_name = format!("str_init_{}", name);
                        let mut bytes = value.as_bytes().to_vec();
                        bytes.push(0); // null-terminated

                        let const_str = context.const_string(&bytes, false);
                        let global = module.add_global(
                            context.i8_type().array_type(bytes.len() as u32),
                            None,
                            &string_name,
                        );
                        global.set_initializer(&const_str);
                        global.set_linkage(Linkage::Private);
                        global.set_constant(true);

                        let zero = context.i32_type().const_zero();
                        let indices = [zero, zero];
                        let gep = builder.build_gep(global.as_pointer_value(), &indices, "str_gep").unwrap();

                        let _ = builder.build_store(alloca, gep);
                    }
                    (Expression::AddressOf(inner_expr), BasicTypeEnum::PointerType(_)) => {
                        match &**inner_expr {
                            Expression::Variable(var_name) => {
                                let ptr = variables.get(var_name)
                                    .unwrap_or_else(|| panic!("Variable {} not found", var_name));
                                builder.build_store(alloca, ptr.ptr).unwrap();
                            }
                            Expression::ArrayLiteral(elements) => {
                                let elem_type = match llvm_type {
                                    BasicTypeEnum::PointerType(ptr_ty) => {
                                        match ptr_ty.get_element_type() {
                                            AnyTypeEnum::ArrayType(arr_ty) => arr_ty.get_element_type(),
                                            _ => panic!("Expected pointer to array type"),
                                        }
                                    }
                                    _ => panic!("Expected pointer-to-array type for array literal"),
                                };

                                let array_type = elem_type.array_type(elements.len() as u32);
                                let tmp_alloca = builder.build_alloca(array_type, "tmp_array").unwrap();

                                for (i, expr) in elements.iter().enumerate() {
                                    let val = generate_expression_ir(
                                        context,
                                        builder,
                                        expr,
                                        variables,
                                        module,
                                        Some(elem_type),
                                    );
                                    let gep = builder.build_in_bounds_gep(
                                        tmp_alloca,
                                        &[
                                            context.i32_type().const_zero(),
                                            context.i32_type().const_int(i as u64, false),
                                        ],
                                        &format!("array_idx_{}", i),
                                    ).unwrap();
                                    builder.build_store(gep, val).unwrap();
                                }

                                builder.build_store(alloca, tmp_alloca).unwrap();
                            }
                            _ => panic!("& operator must be used on variable name or array literal"),
                        }
                    }
                    (Expression::Deref(inner_expr), BasicTypeEnum::IntType(int_type)) => {
                        let target_ptr = match &**inner_expr {
                            Expression::Variable(var_name) => {
                                let ptr_to_value = variables.get(var_name).unwrap().ptr;
                                builder.build_load(ptr_to_value, "load_ptr").unwrap().into_pointer_value()
                            }
                            _ => panic!("Invalid deref in variable init"),
                        };

                        let val = builder.build_load(target_ptr, "deref_value").unwrap();
                        let _ = builder.build_store(alloca, val);
                    }
                    (Expression::IndexAccess { target, index }, _) => {
                        let val = generate_expression_ir(context, builder, init, variables, module, Some(llvm_type));
                        builder.build_store(alloca, val).unwrap();
                    }
                    (Expression::FunctionCall { .. }, _) => {
                        let val = generate_expression_ir(context, builder, init, variables, module, Some(llvm_type));
                        builder.build_store(alloca, val).unwrap();
                    }
                    (Expression::BinaryExpression { .. }, _) => {
                        let val = generate_expression_ir(context, builder, init, variables, module, Some(llvm_type));

                        let casted_val = match (val, llvm_type) {
                            (BasicValueEnum::FloatValue(v), BasicTypeEnum::IntType(t)) => {
                                builder.build_float_to_signed_int(v, t, "float_to_int").unwrap().as_basic_value_enum()
                            }
                            (BasicValueEnum::IntValue(v), BasicTypeEnum::FloatType(t)) => {
                                builder.build_signed_int_to_float(v, t, "int_to_float").unwrap().as_basic_value_enum()
                            }
                            _ => val,
                        };

                        builder.build_store(alloca, casted_val).unwrap();
                    }
                    (Expression::Variable(var_name), _) => {
                        let source_var = variables.get(var_name)
                            .unwrap_or_else(|| panic!("Variable {} not found", var_name));

                        let loaded_value = builder
                            .build_load(source_var.ptr, &format!("load_{}", var_name))
                            .unwrap();

                        let loaded_type = loaded_value.get_type();

                        let casted_value = match (loaded_type, llvm_type) {
                            (BasicTypeEnum::IntType(_), BasicTypeEnum::FloatType(float_ty)) => {
                                builder.build_signed_int_to_float(
                                    loaded_value.into_int_value(),
                                    float_ty,
                                    "int_to_float",
                                ).unwrap().as_basic_value_enum()
                            }
                            (BasicTypeEnum::FloatType(_), BasicTypeEnum::IntType(int_ty)) => {
                                builder.build_float_to_signed_int(
                                    loaded_value.into_float_value(),
                                    int_ty,
                                    "float_to_int",
                                ).unwrap().as_basic_value_enum()
                            }
                            _ => loaded_value,
                        };

                        builder.build_store(alloca, casted_value).unwrap();
                    }
                    (Expression::AsmBlock { instructions, inputs, outputs }, BasicTypeEnum::IntType(int_type)) => {
                        use inkwell::InlineAsmDialect;
                        use inkwell::values::{BasicMetadataValueEnum, CallableValue};

                        let asm_code: String = instructions.join("\n");
                        let mut operand_vals: Vec<BasicMetadataValueEnum> = vec![];
                        let mut constraint_parts = vec![];

                        for (reg, var) in inputs {
                            let val = if let Ok(num) = var.parse::<i64>() {
                                context.i64_type().const_int(num as u64, false).into()
                            } else if let Some(info) = variables.get(var) {
                                builder.build_load(info.ptr, var).unwrap().into()
                            } else {
                                panic!("Input variable '{}' not found", var);
                            };

                            operand_vals.push(val);
                            constraint_parts.push(format!("{{{}}}", reg));
                        }

                        for (reg, _) in outputs {
                            constraint_parts.insert(0, format!("={{{}}}", reg));
                        }

                        let constraint_str = constraint_parts.join(",");

                        let (fn_type, expects_return) = if outputs.is_empty() {
                            (context.void_type().fn_type(&[], false), false)
                        } else {
                            (context.i64_type().fn_type(&[], false), true)
                        };

                        let inline_asm_ptr = context.create_inline_asm(
                            fn_type,
                            asm_code,
                            constraint_str,
                            true,  // has_side_effects
                            false, // align_stack
                            Some(InlineAsmDialect::Intel),
                            false, // can_throw
                        );

                        let inline_asm_fn = CallableValue::try_from(inline_asm_ptr)
                            .expect("Failed to cast inline asm to CallableValue");

                        let call = builder
                            .build_call(inline_asm_fn, &operand_vals, "inline_asm")
                            .unwrap();

                        if expects_return {
                            let result = call.try_as_basic_value().left()
                                .expect("Expected return value from inline asm but got none");

                            builder.build_store(alloca, result).unwrap();
                        }
                    }
                    _ => {
                        panic!("Unsupported type/value combination for initialization: {:?}", init);
                    }
                }
            }
        }
        ASTNode::Statement(StatementNode::Println(message)) |
        ASTNode::Statement(StatementNode::Print(message)) => {
            let global_name = format!("str_{}", *string_counter);
            *string_counter += 1;

            let mut bytes = message.as_bytes().to_vec();
            bytes.push(0);
            let const_str = context.const_string(&bytes, false);

            let global = module.add_global(
                context.i8_type().array_type(bytes.len() as u32),
                None,
                &global_name,
            );
            global.set_initializer(&const_str);
            global.set_linkage(Linkage::Private);
            global.set_constant(true);

            let printf_type = context.i32_type().fn_type(
                &[context.i8_type().ptr_type(AddressSpace::default()).into()],
                true,
            );
            let printf_func = match module.get_function("printf") {
                Some(f) => f,
                None => module.add_function("printf", printf_type, None),
            };

            let zero = context.i32_type().const_zero();
            let indices = [zero, zero];
            let gep = unsafe {
                builder.build_gep(global.as_pointer_value(), &indices, "gep").unwrap()
            };

            let _ = builder.build_call(printf_func, &[gep.into()], "printf_call");
        }
        ASTNode::Statement(StatementNode::PrintlnFormat { format, args }) |
        ASTNode::Statement(StatementNode::PrintFormat { format, args }) => {
            let mut arg_types = vec![];
            for arg in args {
                let val = generate_expression_ir(context, builder, arg, variables, module, None);
                arg_types.push(val.get_type());
            }
            let c_format_string = wave_format_to_c(&format, &arg_types);

            let global_name = format!("str_{}", *string_counter);
            *string_counter += 1;

            let mut bytes = c_format_string.as_bytes().to_vec();
            bytes.push(0);
            let const_str = context.const_string(&bytes, false);

            let global = module.add_global(
                context.i8_type().array_type(bytes.len() as u32),
                None,
                &global_name,
            );
            global.set_initializer(&const_str);
            global.set_linkage(Linkage::Private);
            global.set_constant(true);

            let printf_type = context.i32_type().fn_type(
                &[context.i8_type().ptr_type(AddressSpace::default()).into()],
                true,
            );
            let printf_func = match module.get_function("printf") {
                Some(func) => func,
                None => module.add_function("printf", printf_type, None),
            };

            let zero = context.i32_type().const_zero();
            let indices = [zero, zero];
            let gep = unsafe {
                builder.build_gep(global.as_pointer_value(), &indices, "gep").unwrap()
            };

            let mut printf_args = vec![gep.into()];
            for arg in args {
                let value = generate_expression_ir(context, builder, arg, variables, module, None);

                let casted_value = match value {
                    BasicValueEnum::PointerValue(ptr_val) => {
                        let element_ty = ptr_val.get_type().get_element_type();
                        if element_ty.is_int_type() && element_ty.into_int_type().get_bit_width() == 8 {
                            ptr_val.as_basic_value_enum()
                        } else {
                            builder
                                .build_ptr_to_int(ptr_val, context.i64_type(), "ptr_as_int")
                                .unwrap()
                                .as_basic_value_enum()
                        }
                    }
                    BasicValueEnum::FloatValue(fv) => {
                        let double_ty = context.f64_type();
                        builder
                            .build_float_ext(fv, double_ty, "cast_to_double")
                            .unwrap()
                            .as_basic_value_enum()
                    }
                    _ => value,
                };

                printf_args.push(casted_value.into());
            }

            let _ = builder.build_call(printf_func, &printf_args, "printf_call");
        }
        ASTNode::Statement(StatementNode::If {
                               condition,
                               body,
                               else_if_blocks,
                               else_block,
                           }) => {
            let current_fn = builder.get_insert_block().unwrap().get_parent().unwrap();

            let cond_value = generate_expression_ir(context, builder, condition, variables, module, None);

            let then_block = context.append_basic_block(current_fn, "then");
            let else_block_bb = context.append_basic_block(current_fn, "else");
            let merge_block = context.append_basic_block(current_fn, "merge");

            let _ = builder.build_conditional_branch(cond_value.try_into().unwrap(), then_block, else_block_bb);

            let then_has_terminator;
            let else_has_terminator;

            // then
            builder.position_at_end(then_block);
            for stmt in body {
                generate_statement_ir(context, builder, module, string_counter, stmt, variables, loop_exit_stack, loop_continue_stack, current_function);
            }
            then_has_terminator = then_block.get_terminator().is_some();
            if !then_has_terminator {
                let _ = builder.build_unconditional_branch(merge_block);
            }

            // else
            builder.position_at_end(else_block_bb);

            if let Some(else_ifs) = else_if_blocks {
                for else_if in else_ifs.iter() {
                    generate_statement_ir(context, builder, module, string_counter, else_if, variables, loop_exit_stack, loop_continue_stack, current_function);
                }
            }

            if let Some(else_body) = else_block {
                for stmt in else_body.iter() {
                    generate_statement_ir(context, builder, module, string_counter, stmt, variables, loop_exit_stack, loop_continue_stack, current_function);
                }
            }
            else_has_terminator = else_block_bb.get_terminator().is_some();
            if !else_has_terminator {
                let _ = builder.build_unconditional_branch(merge_block);
            }

            if !then_has_terminator || !else_has_terminator {
                builder.position_at_end(merge_block);
            }
        }
        ASTNode::Statement(StatementNode::While { condition, body }) => {
            let current_fn = builder.get_insert_block().unwrap().get_parent().unwrap();

            let cond_block = context.append_basic_block(current_fn, "while.cond");
            let body_block = context.append_basic_block(current_fn, "while.body");
            let merge_block = context.append_basic_block(current_fn, "while.end");

            loop_exit_stack.push(merge_block);
            loop_continue_stack.push(cond_block);

            let _ = builder.build_unconditional_branch(cond_block);
            builder.position_at_end(cond_block);

            let cond_val = generate_expression_ir(context, builder, condition, variables, module, None);

            let cond_bool = match cond_val {
                BasicValueEnum::IntValue(val) => {
                    let zero = val.get_type().const_zero();
                    builder
                        .build_int_compare(inkwell::IntPredicate::NE, val, zero, "while_cond")
                        .unwrap()
                }
                BasicValueEnum::FloatValue(val) => {
                    let zero = val.get_type().const_float(0.0);
                    builder
                        .build_float_compare(FloatPredicate::ONE, val, zero, "while_cond")
                        .unwrap()
                }
                _ => panic!("Unsupported condition type in while loop"),
            };

            let _ = builder.build_conditional_branch(cond_bool, body_block, merge_block);

            builder.position_at_end(body_block);
            for stmt in body.iter() {
                generate_statement_ir(context, builder, module, string_counter, stmt, variables, loop_exit_stack, loop_continue_stack, current_function);
            }
            let _ = builder.build_unconditional_branch(cond_block);

            loop_exit_stack.pop();
            loop_continue_stack.pop();

            builder.position_at_end(merge_block);
        }
        ASTNode::Statement(StatementNode::AsmBlock { instructions, inputs, outputs }) => {
            use inkwell::InlineAsmDialect;
            use inkwell::values::{BasicMetadataValueEnum, CallableValue};
            use std::collections::HashSet;

            let asm_code: String = instructions.join("\n");
            let mut operand_vals: Vec<BasicMetadataValueEnum> = vec![];
            let mut constraint_parts: Vec<String> = vec![];

            let input_regs: HashSet<_> = inputs.iter().map(|(reg, _)| reg.to_string()).collect();
            let mut seen_regs: HashSet<String> = HashSet::new();

            for (reg, var) in outputs {
                if input_regs.contains(reg) {
                    panic!("Register '{}' used in both input and output in inline asm", reg);
                }

                if !seen_regs.insert(reg.to_string()) {
                    panic!("Register '{}' duplicated in outputs", reg);
                }

                let info = variables
                    .get(var)
                    .unwrap_or_else(|| panic!("Output variable '{}' not found", var));
                let dummy_val = builder.build_load(info.ptr, var).unwrap().into();
                operand_vals.push(dummy_val);
                constraint_parts.push(format!("={{{}}}", reg)); // e.g., ={rax}
            }

            for (reg, var) in inputs {
                if !seen_regs.insert(reg.to_string()) {
                    panic!("Register '{}' duplicated in inputs", reg);
                }

                let val: BasicMetadataValueEnum = if let Ok(value) = var.parse::<i64>() {
                    context.i64_type().const_int(value as u64, false).into()
                } else {
                    let info = variables
                        .get(var)
                        .unwrap_or_else(|| panic!("Input variable '{}' not found", var));
                    builder.build_load(info.ptr, var).unwrap().into()
                };

                operand_vals.push(val);
                constraint_parts.push(format!("{{{}}}", reg));
            }

            let constraints_str: String = constraint_parts.join(",");

            let fn_type = context.i64_type().fn_type(&[], false);

            let inline_asm_ptr = context.create_inline_asm(
                fn_type,
                asm_code,
                constraints_str,
                true,
                false,
                Some(InlineAsmDialect::Intel),
                false,
            );

            let inline_asm_fn = CallableValue::try_from(inline_asm_ptr)
                .expect("Failed to convert inline asm to CallableValue");

            let call = builder
                .build_call(inline_asm_fn, &operand_vals, "inline_asm")
                .unwrap();

            if let Some((_, out_var)) = outputs.first() {
                let ret_ptr = variables.get(out_var).unwrap().ptr;
                let ret_val = call.try_as_basic_value().left().unwrap();
                builder.build_store(ret_ptr, ret_val).unwrap();
            }
        }
        ASTNode::Statement(StatementNode::Expression(expr)) => {
            let _ = generate_expression_ir(context, builder, expr, variables, module, None);
        }
        ASTNode::Statement(StatementNode::Assign { variable, value }) => {
            if variable == "deref" {
                if let Expression::BinaryExpression { left, operator: _, right } = value {
                    if let Expression::Deref(inner_expr) = &**left {
                        let target_ptr = generate_address_ir(context, builder, inner_expr, variables, module);
                        let val = generate_expression_ir(context, builder, right, variables, module, None);
                        builder.build_store(target_ptr, val).unwrap();
                    }
                }
                return;
            }

            let var_info = variables.get(variable)
                .unwrap_or_else(|| panic!("Variable {} not declared", variable));

            if matches!(var_info.mutability, Mutability::Let) {
                panic!("Cannot assign to immutable variable '{}'", variable);
            }

            let element_type = var_info.ptr.get_type().get_element_type();

            let expected_type = match element_type {
                AnyTypeEnum::IntType(t) => t.as_basic_type_enum(),
                AnyTypeEnum::FloatType(t) => t.as_basic_type_enum(),
                AnyTypeEnum::PointerType(t) => t.as_basic_type_enum(),
                AnyTypeEnum::ArrayType(t) => t.as_basic_type_enum(),
                AnyTypeEnum::StructType(t) => t.as_basic_type_enum(),
                AnyTypeEnum::VectorType(t) => t.as_basic_type_enum(),
                _ => panic!("Unsupported LLVM type in assignment"),
            };

            let val = generate_expression_ir(context, builder, value, variables, module, Some(expected_type));

            if let Some(var_info) = variables.get(variable) {
                if matches!(var_info.mutability, Mutability::Let) {
                    panic!("Cannot assign to immutable variable '{}'", variable);
                }

                let element_type = match var_info.ptr.get_type().get_element_type() {
                    AnyTypeEnum::IntType(t) => BasicTypeEnum::IntType(t),
                    AnyTypeEnum::FloatType(t) => BasicTypeEnum::FloatType(t),
                    AnyTypeEnum::PointerType(t) => BasicTypeEnum::PointerType(t),
                    AnyTypeEnum::ArrayType(t) => BasicTypeEnum::ArrayType(t),
                    AnyTypeEnum::StructType(t) => BasicTypeEnum::StructType(t),
                    AnyTypeEnum::VectorType(t) => BasicTypeEnum::VectorType(t),
                    _ => panic!("Unsupported LLVM type in assignment"),
                };

                let casted_val = match (val, element_type) {
                    (BasicValueEnum::FloatValue(v), BasicTypeEnum::IntType(t)) => {
                        builder.build_float_to_signed_int(v, t, "float_to_int").unwrap().as_basic_value_enum()
                    }
                    (BasicValueEnum::IntValue(v), BasicTypeEnum::FloatType(t)) => {
                        builder.build_signed_int_to_float(v, t, "int_to_float").unwrap().as_basic_value_enum()
                    }
                    _ => val,
                };
                builder.build_store(var_info.ptr, casted_val).unwrap();
            } else {
                panic!("Variable {} not declared", variable);
            }
        }
        ASTNode::Statement(StatementNode::Break) => {
            if let Some(target_block) = loop_exit_stack.last() {
                let _ = builder.build_unconditional_branch(*target_block);
            } else {
                panic!("break used outside of loop!");
            }
        }
        ASTNode::Statement(StatementNode::Continue) => {
            if let Some(target_block) = loop_continue_stack.last() {
                let _ = builder.build_unconditional_branch(*target_block);
                builder.build_unreachable().unwrap();
            } else {
                panic!("continue used outside of loop!");
            }
        }
        ASTNode::Statement(StatementNode::Return(expr_opt)) => {
            if let Some(expr) = expr_opt {
                let ret_type = current_function.get_type().get_return_type()
                    .expect("Function should have a return type");
                let expected_type = ret_type.try_into()
                    .expect("Failed to convert return type to BasicTypeEnum");

                let value = generate_expression_ir(
                    context,
                    builder,
                    expr,
                    variables,
                    module,
                    Some(expected_type),
                );

                let value = match value {
                    BasicValueEnum::PointerValue(ptr) => {
                        builder.build_load(ptr, "load_ret").unwrap().as_basic_value_enum()
                    },
                    other => other,
                };

                let casted_value = match (value, expected_type) {
                    (BasicValueEnum::FloatValue(v), BasicTypeEnum::IntType(t)) => {
                        builder.build_float_to_signed_int(v, t, "float_to_int").unwrap().as_basic_value_enum()
                    }
                    (BasicValueEnum::IntValue(v), BasicTypeEnum::FloatType(t)) => {
                        builder.build_signed_int_to_float(v, t, "int_to_float").unwrap().as_basic_value_enum()
                    }
                    _ => value,
                };

                let _ = builder.build_return(Some(&casted_value));
            } else {
                let _ = builder.build_return(None);
            }
        }
        ASTNode::Statement(StatementNode::Expression(expr)) => {
            generate_expression_ir(context, builder, expr, variables, module, None);
        }
        _ => {}
    }
}